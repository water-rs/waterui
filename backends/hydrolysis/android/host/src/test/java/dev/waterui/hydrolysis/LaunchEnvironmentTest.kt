package dev.waterui.hydrolysis

import android.content.Context
import android.content.pm.ApplicationInfo
import android.content.pm.ProviderInfo
import android.net.Uri
import android.os.Looper
import android.os.ParcelFileDescriptor
import java.io.File
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.nio.file.attribute.PosixFilePermissions
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.internal.DoNotInstrument
import org.robolectric.shadows.ShadowBinder

/** A context that counts `noBackupFilesDir` lookups — an invalid id must
 *  refuse before the filesystem is even asked for a path. */
private class CountingContext(base: Context) : android.content.ContextWrapper(base) {
    var noBackupLookups = 0

    override fun getNoBackupFilesDir(): File {
        noBackupLookups += 1
        return super.getNoBackupFilesDir()
    }
}

/** A JVM [PendingDescriptorFactory]: NIO atomic owner-only create plus a
 *  real write-only descriptor, and a call count denial tests read. */
private class FixtureDescriptorFactory : PendingDescriptorFactory {
    val calls = AtomicInteger()

    override fun create(dir: File): ParcelFileDescriptor {
        calls.incrementAndGet()
        val pending = LaunchEnvironmentStore.pendingFile(dir)
        Files.createFile(
            pending.toPath(),
            PosixFilePermissions.asFileAttribute(
                setOf(
                    java.nio.file.attribute.PosixFilePermission.OWNER_READ,
                    java.nio.file.attribute.PosixFilePermission.OWNER_WRITE,
                )
            ),
        )
        return ParcelFileDescriptor.open(pending, ParcelFileDescriptor.MODE_WRITE_ONLY)
    }
}

/**
 * The launch-environment handoff's device half: the provider's
 * authorisation, one-shot allocation and write; and the store's claim
 * lifecycle and strict payload rules. Dummy values stand in for real
 * declarations — the fixture never touches the JVM's own environment.
 */
@RunWith(RobolectricTestRunner::class)
@DoNotInstrument
class LaunchEnvironmentStoreTest {
    private val context: Context = RuntimeEnvironment.getApplication()

    private val id = "1d50c8a0-9f3f-4a56-8b7e-0a1b2c3d4e5f"
    private val payload = "{\"WATERUI_DUMMY_KEY\":\"dummy-value\"}"

    private lateinit var factory: FixtureDescriptorFactory
    private lateinit var provider: LaunchEnvironmentProvider

    private fun environmentUri(environmentId: String = id): Uri =
        Uri.parse("content://com.example.app.waterui.launch-environment/$environmentId")

    @Before
    fun reset() {
        factory = FixtureDescriptorFactory()
        provider = LaunchEnvironmentProvider(factory)
        provider.attachInfo(
            context,
            ProviderInfo().apply {
                authority = "com.example.app.waterui.launch-environment"
                packageName = context.packageName
                name = LaunchEnvironmentProvider::class.java.name
            },
        )
        ShadowBinder.setCallingUid(android.os.Process.myUid())
        LaunchEnvironmentStore.root(context).deleteRecursively()
    }

    private fun writePending(environmentId: String = id, body: String = payload) {
        provider.openFile(environmentUri(environmentId), "w").use { descriptor ->
            java.io.FileOutputStream(descriptor.fileDescriptor).use {
                it.write(body.toByteArray(StandardCharsets.UTF_8))
            }
        }
    }

    /** Stage the pending payload without the provider — claim-only tests.
     *  The file carries the same owner-only birth permissions the real
     *  descriptor factory creates. */
    private fun stagePending(environmentId: String = id, body: String = payload) {
        val dir = LaunchEnvironmentStore.allocateId(context, environmentId)
        val pending = LaunchEnvironmentStore.pendingFile(dir)
        Files.createFile(
            pending.toPath(),
            PosixFilePermissions.asFileAttribute(
                setOf(
                    java.nio.file.attribute.PosixFilePermission.OWNER_READ,
                    java.nio.file.attribute.PosixFilePermission.OWNER_WRITE,
                )
            ),
        )
        Files.write(pending.toPath(), body.toByteArray(StandardCharsets.UTF_8))
    }

    private fun claimedMap(values: LaunchEnvironmentValues): Map<String, String> {
        val map = LinkedHashMap<String, String>()
        values.applyTo(true) { key, value -> map[key] = value }
        return map
    }

    @Test
    fun theOwnProcessIsAuthorised() {
        writePending()
        assertEquals(1, factory.calls.get())
        assertTrue(
            LaunchEnvironmentStore.pendingFile(
                LaunchEnvironmentStore.idDirectory(context, id),
            ).exists()
        )
    }

    @Test
    fun theShellIsAuthorised() {
        val shellUid = 55_123
        shadowOf(context.packageManager).setPackagesForUid(shellUid, "com.android.shell")
        ShadowBinder.setCallingUid(shellUid)
        writePending()
        assertEquals(1, factory.calls.get())
    }

    @Test
    fun anUnrelatedCallerIsDeniedBeforeAnyFile() {
        val foreignUid = 60_001
        shadowOf(context.packageManager).setPackagesForUid(foreignUid, "com.foreign.app")
        ShadowBinder.setCallingUid(foreignUid)
        try {
            writePending()
            fail("an unrelated uid must not write")
        } catch (expected: SecurityException) {
        }
        // Denied before the filesystem: no namespace, no per-id dir, and
        // the descriptor factory never ran.
        assertFalse(LaunchEnvironmentStore.root(context).exists())
        assertEquals(0, factory.calls.get())
    }

    @Test
    fun readPathsAreNotImplemented() {
        val uri = environmentUri()
        for (call in listOf<() -> Unit>(
            { provider.query(uri, null, null, null, null) },
            { provider.insert(uri, null) },
            { provider.update(uri, null, null, null) },
            { provider.getType(uri) },
            { provider.call("anything", null, null) },
        )) {
            try {
                call()
                fail("the handoff is write-only")
            } catch (expected: UnsupportedOperationException) {
            }
        }
    }

    @Test
    fun anIdDirectoryIsAllocatedExactlyOnce() {
        writePending()
        // A second write against the same id — pending still there —
        // refuses before a descriptor exists.
        try {
            writePending(body = "{\"OTHER\":\"x\"}")
            fail("the per-id directory is one-shot")
        } catch (expected: java.io.FileNotFoundException) {
        }
        assertEquals(1, factory.calls.get())
        assertEquals(
            payload,
            LaunchEnvironmentStore.pendingFile(
                LaunchEnvironmentStore.idDirectory(context, id),
            ).readText(),
        )
    }

    @Test
    fun anIdClaimedActiveStillRefusesReallocation() {
        writePending()
        LaunchEnvironmentStore.claim(context, id)
        try {
            provider.openFile(environmentUri(), "w")
            fail("a claimed id cannot be reallocated")
        } catch (expected: java.io.FileNotFoundException) {
        }
        assertEquals(1, factory.calls.get())
    }

    @Test
    fun aSymlinkAtThePerIdPathIsNotFollowed() {
        val root = Files.createDirectory(
            LaunchEnvironmentStore.root(context).toPath(),
            PosixFilePermissions.asFileAttribute(
                PosixFilePermissions.fromString("rwx------"),
            ),
        )
        assertEquals(
            PosixFilePermissions.fromString("rwx------"),
            Files.getPosixFilePermissions(root),
        )
        val target = Files.createDirectory(
            File(context.noBackupFilesDir, "elsewhere").toPath(),
            PosixFilePermissions.asFileAttribute(
                PosixFilePermissions.fromString("rwx------"),
            ),
        ).toFile()
        val marker = File(target, "marker")
        marker.writeText("sentinel")
        Files.createSymbolicLink(
            LaunchEnvironmentStore.idDirectory(context, id).toPath(),
            target.toPath(),
        )
        try {
            writePending()
            fail("a symlink must never become the per-id directory")
        } catch (expected: java.io.FileNotFoundException) {
        }
        assertEquals(0, factory.calls.get())
        // The symlink's target is untouched — nothing was written or
        // truncated through the link.
        assertEquals("sentinel", marker.readText())
        assertFalse(File(target, "pending").exists())
    }

    @Test
    fun filesAndDirectoriesAreBornOwnerOnly() {
        writePending()
        val dir = LaunchEnvironmentStore.idDirectory(context, id)
        assertEquals(
            PosixFilePermissions.fromString("rwx------"),
            Files.getPosixFilePermissions(dir.toPath()),
        )
        assertEquals(
            PosixFilePermissions.fromString("rwx------"),
            Files.getPosixFilePermissions(LaunchEnvironmentStore.root(context).toPath()),
        )
        assertEquals(
            PosixFilePermissions.fromString("rw-------"),
            Files.getPosixFilePermissions(
                LaunchEnvironmentStore.pendingFile(dir).toPath(),
            ),
        )
    }

    @Test
    fun anIdOutsideTheCanonicalShapeIsRefusedBeforeAnyFilesystem() {
        val counting = CountingContext(context)
        for (bad in listOf(
            "..",
            "not-a-uuid",
            "ABCDEF01-0000-0000-0000-000000000000",
            "1d50c8a0-9f3f-4a56-8b7e-0a1b2c3d4e5f/extra",
            "1d50c8a0 9f3f-4a56-8b7e-0a1b2c3d4e5f",
        )) {
            try {
                provider.openFile(environmentUri(bad), "w")
                fail("'$bad' is not a valid handoff id")
            } catch (expected: Exception) {
            }
            try {
                LaunchEnvironmentStore.claim(counting, bad)
                fail("'$bad' is not a valid handoff id")
            } catch (expected: Exception) {
            }
            try {
                provider.delete(environmentUri(bad), null, null)
                fail("'$bad' is not a valid handoff id")
            } catch (expected: Exception) {
            }
        }
        // Every entrance validated the raw id before asking the context
        // for a single path — no root exists, no factory ran, and the
        // counting context never saw a directory lookup.
        assertFalse(LaunchEnvironmentStore.root(context).exists())
        assertEquals(0, factory.calls.get())
        assertEquals(0, counting.noBackupLookups)
    }

    @Test
    fun claimMovesPendingToActiveAndReadsAgain() {
        stagePending()
        val claimed = LaunchEnvironmentStore.claim(context, id)
        assertEquals(
            mapOf("WATERUI_DUMMY_KEY" to "dummy-value"),
            claimedMap(claimed),
        )
        val dir = LaunchEnvironmentStore.idDirectory(context, id)
        assertFalse(LaunchEnvironmentStore.pendingFile(dir).exists())
        assertTrue(LaunchEnvironmentStore.activeFile(dir).exists())

        // A recreated process carrying the same id reads the ACTIVE config
        // again — the retained state, not a fallback.
        assertEquals(claimedMap(claimed), claimedMap(LaunchEnvironmentStore.claim(context, id)))
    }

    @Test
    fun conflictingAndAbsentIdsFailClearly() {
        stagePending()
        LaunchEnvironmentStore.claim(context, id)
        val dir = LaunchEnvironmentStore.idDirectory(context, id)
        Files.write(LaunchEnvironmentStore.pendingFile(dir).toPath(), "{}".toByteArray())
        try {
            LaunchEnvironmentStore.claim(context, id)
            fail("pending plus active is conflicting state")
        } catch (expected: IllegalStateException) {
        }
        Files.delete(LaunchEnvironmentStore.pendingFile(dir).toPath())

        try {
            LaunchEnvironmentStore.claim(context, "00000000-0000-0000-0000-000000000000")
            fail("an absent id cannot claim")
        } catch (expected: IllegalStateException) {
        }
    }

    @Test
    fun aPendingOrActiveSymlinkIsNeverRead() {
        val target = File(context.noBackupFilesDir, "outside.json")
        target.writeText("{\"LEAK\":\"yes\"}")
        val dir = LaunchEnvironmentStore.allocateId(context, id)
        Files.createSymbolicLink(
            LaunchEnvironmentStore.pendingFile(dir).toPath(),
            target.toPath(),
        )
        try {
            LaunchEnvironmentStore.claim(context, id)
            fail("a symlink payload must not be read")
        } catch (expected: IllegalStateException) {
        }
        assertEquals("{\"LEAK\":\"yes\"}", target.readText())

        // Same invariant on the ACTIVE slot: a foreign file linked in is
        // still foreign.
        Files.delete(LaunchEnvironmentStore.pendingFile(dir).toPath())
        Files.createSymbolicLink(
            LaunchEnvironmentStore.activeFile(dir).toPath(),
            target.toPath(),
        )
        try {
            LaunchEnvironmentStore.claim(context, id)
            fail("a symlink active slot must not be read")
        } catch (expected: IllegalStateException) {
        }
    }

    @Test
    fun malformedPayloadsAreRejectedWithoutEchoingThem() {
        val sentinel = "s3ntinel-value"
        for (bad in listOf(
            "{",
            "{\"K\":\"v\"} trailing",
            "{\"K\":\"v\",}",
            "{\"K\":1}",
            "{\"K\":null}",
            "{\"K\":{}}",
            "{\"K\":[\"a\"]}",
            "{\"K=NO\":\"v\"}",
            "{\"\":\"v\"}",
            "{\"K\":\"1\",\"K\":\"2\"}",
            "{\"K\\u0000X\":\"v\"}",
            "{\"K\":\"v\\u0000x\"}",
            "{\"K\":\"\\ud800\"}",
            "{\"\\ud800\":\"v\"}",
            "{\"\\ud800x\":\"v\"}",
            "[ ]",
            "{\"S\":\"$sentinel\",BROKEN",
        )) {
            stagePending(body = bad)
            try {
                LaunchEnvironmentStore.claim(context, id)
                fail("'$bad' must not parse")
            } catch (expected: IllegalStateException) {
                // Every malformed row collapses to the one fixed message —
                // the platform parser's own diagnostics never surface.
                assertEquals(
                    "the launch environment is not a valid JSON object",
                    expected.message,
                )
                assertNull(expected.cause)
                val chain = generateSequence<Throwable>(expected) { it.cause }
                for (throwable in chain) {
                    assertFalse(
                        "the error never echoes the body: ${throwable.message}",
                        throwable.message.orEmpty().contains(sentinel),
                    )
                    assertFalse(
                        throwable.toString().contains("dummy-value")
                            || throwable.toString().contains(sentinel),
                    )
                }
            }
            LaunchEnvironmentStore.root(context).deleteRecursively()
        }
    }

    @Test
    fun validOuterWhitespaceAndOpaqueValuesRoundTrip() {
        stagePending(body = "  { \"A\" : \"x\", \"B\" : \"a  b\\nc\", \"C\" : \"x=y\", \"D\" : \"\", \"E\" : \"日本語\" }\n")
        val claimed = LaunchEnvironmentStore.claim(context, id)
        assertEquals(
            linkedMapOf(
                "A" to "x",
                "B" to "a  b\nc",
                "C" to "x=y",
                "D" to "",
                "E" to "日本語",
            ),
            claimedMap(claimed),
        )
    }

    @Test
    fun invalidUtf8IsRejected() {
        stagePending(body = "{}")
        Files.write(
            LaunchEnvironmentStore.pendingFile(
                LaunchEnvironmentStore.idDirectory(context, id),
            ).toPath(),
            byteArrayOf(0x78, 0xff.toByte()),
            java.nio.file.StandardOpenOption.TRUNCATE_EXISTING,
        )
        try {
            LaunchEnvironmentStore.claim(context, id)
            fail("invalid UTF-8 must not parse")
        } catch (expected: IllegalStateException) {
            // The strict-decoder verdict, not a filesystem policy failure.
            assertEquals(
                "the launch environment is not a valid JSON object",
                expected.message,
            )
            assertNull(expected.cause)
        }
    }

    @Test
    fun twoConcurrentReadersBothGetTheSameClaimedValues() {
        stagePending()
        val entered = CountDownLatch(1)
        val release = CountDownLatch(1)
        var first: LaunchEnvironmentValues? = null
        var firstError: Throwable? = null
        val firstReader = Thread {
            try {
                first = LaunchEnvironmentStore.claim(context, id) {
                    entered.countDown()
                    release.await(5, TimeUnit.SECONDS)
                }
            } catch (error: Throwable) {
                firstError = error
            }
        }
        firstReader.start()
        assertTrue(entered.await(5, TimeUnit.SECONDS))

        // The second reader claims fully while the first is parked between
        // its pending read and its move.
        val second = LaunchEnvironmentStore.claim(context, id)
        release.countDown()
        firstReader.join(5_000)

        assertNull(firstError?.let { "first reader failed: $it" })
        assertEquals(claimedMap(second), claimedMap(first!!))
        val dir = LaunchEnvironmentStore.idDirectory(context, id)
        assertTrue(LaunchEnvironmentStore.activeFile(dir).exists())
    }

    @Test
    fun aSymlinkedRootNamespaceIsRefusedEverywhere() {
        Files.createDirectories(LaunchEnvironmentStore.root(context).toPath().parent)
        // The link's target is itself a private-looking directory: the
        // refusal can only come from not following the link, never from
        // the target's permissions.
        val target = Files.createDirectory(
            File(context.noBackupFilesDir, "foreign-root").toPath(),
            PosixFilePermissions.asFileAttribute(
                PosixFilePermissions.fromString("rwx------"),
            ),
        ).toFile()
        val marker = File(target, "marker")
        marker.writeText("sentinel")
        Files.createSymbolicLink(LaunchEnvironmentStore.root(context).toPath(), target.toPath())

        try {
            provider.openFile(environmentUri(), "w")
            fail("a symlinked namespace is not the store")
        } catch (expected: java.io.FileNotFoundException) {
        }
        try {
            LaunchEnvironmentStore.claim(context, id)
            fail("readers never descend a symlinked root")
        } catch (expected: IllegalStateException) {
        }
        try {
            provider.delete(environmentUri(), null, null)
            fail("deleters never descend a symlinked root")
        } catch (expected: IllegalArgumentException) {
        }
        assertEquals(0, factory.calls.get())
        assertEquals("sentinel", marker.readText())
    }

    @Test
    fun anExistingNonprivateRootIsRefusedNotRepaired() {
        val dir = LaunchEnvironmentStore.root(context)
        dir.mkdirs()
        Files.setPosixFilePermissions(
            dir.toPath(),
            PosixFilePermissions.fromString("rwxr-xr-x"),
        )
        // The root already exists without owner-only bits: the store
        // refuses rather than chmodding a foreign directory into place.
        try {
            provider.openFile(environmentUri(), "w")
            fail("a non-private root is not adopted")
        } catch (expected: java.io.FileNotFoundException) {
        }
        assertEquals(0, factory.calls.get())
        assertTrue(dir.exists())
        assertEquals(
            PosixFilePermissions.fromString("rwxr-xr-x"),
            Files.getPosixFilePermissions(dir.toPath()),
        )
    }

    @Test
    fun aSymlinkedIdDirectoryRefusesReadersAndDeleters() {
        stagePending(environmentId = "0a50c8a0-9f3f-4a56-8b7e-0a1b2c3d4e5f")
        val target = LaunchEnvironmentStore.idDirectory(
            context, "0a50c8a0-9f3f-4a56-8b7e-0a1b2c3d4e5f",
        )
        Files.createSymbolicLink(
            LaunchEnvironmentStore.idDirectory(context, id).toPath(),
            target.toPath(),
        )
        try {
            LaunchEnvironmentStore.claim(context, id)
            fail("a symlinked id directory is not the store's slot")
        } catch (expected: IllegalStateException) {
        }
        try {
            provider.delete(environmentUri(), null, null)
            fail("deleters never descend a symlinked id directory")
        } catch (expected: IllegalArgumentException) {
        }
        // The real slot still reads correctly through its own id.
        assertNotNull(
            LaunchEnvironmentStore.claim(
                context, "0a50c8a0-9f3f-4a56-8b7e-0a1b2c3d4e5f",
            ),
        )
    }

    @Test
    fun deleteRemovesOnlyTheExactIdsPayload() {
        writePending()
        assertEquals(1, provider.delete(environmentUri(), null, null))
        val dir = LaunchEnvironmentStore.idDirectory(context, id)
        assertFalse(LaunchEnvironmentStore.pendingFile(dir).exists())
        // The directory stays: the tombstone keeps the id un-reusable.
        assertTrue(dir.exists())
        try {
            provider.openFile(environmentUri(), "w")
            fail("a deleted id stays deleted")
        } catch (expected: java.io.FileNotFoundException) {
        }

        // A selection is never part of the contract.
        try {
            provider.delete(environmentUri(), "x", null)
            fail("no selection")
        } catch (expected: IllegalArgumentException) {
        }
        // An id nobody wrote removes nothing.
        assertEquals(
            0,
            provider.delete(
                environmentUri("00000000-0000-0000-0000-000000000000"),
                null,
                null,
            ),
        )
    }
}

/**
 * The loader's executor boundary and the dev-URL release policy, driven
 * through the store's own pending slot. The instance's injected applier
 * captures into a test-owned map — the JVM's real environment is never
 * touched.
 */
@RunWith(RobolectricTestRunner::class)
@DoNotInstrument
class LaunchEnvironmentLoaderTest {
    private val context: Context = RuntimeEnvironment.getApplication()

    private val id = "1d50c8a0-9f3f-4a56-8b7e-0a1b2c3d4e5f"

    private lateinit var factory: FixtureDescriptorFactory
    private lateinit var provider: LaunchEnvironmentProvider
    private val applied = LinkedHashMap<String, String>()
    private lateinit var loader: LaunchEnvironmentLoader

    @Before
    fun reset() {
        factory = FixtureDescriptorFactory()
        provider = LaunchEnvironmentProvider(factory)
        provider.attachInfo(
            context,
            ProviderInfo().apply {
                authority = "com.example.app.waterui.launch-environment"
                packageName = context.packageName
                name = LaunchEnvironmentProvider::class.java.name
            },
        )
        ShadowBinder.setCallingUid(android.os.Process.myUid())
        LaunchEnvironmentStore.root(context).deleteRecursively()
        applied.clear()
        loader = LaunchEnvironmentLoader(
            context,
            environmentApplier = { key, value -> applied[key] = value },
        )
    }

    private fun stagePending(environmentId: String = id, body: String) {
        val dir = LaunchEnvironmentStore.allocateId(context, environmentId)
        val pending = LaunchEnvironmentStore.pendingFile(dir)
        Files.createFile(
            pending.toPath(),
            PosixFilePermissions.asFileAttribute(
                setOf(
                    java.nio.file.attribute.PosixFilePermission.OWNER_READ,
                    java.nio.file.attribute.PosixFilePermission.OWNER_WRITE,
                )
            ),
        )
        Files.write(pending.toPath(), body.toByteArray(StandardCharsets.UTF_8))
    }

    private fun deliver(id: String?): () -> Unit {
        val latch = CountDownLatch(1)
        var apply: () -> Unit = {}
        loader.prepare(id) {
            apply = it
            latch.countDown()
        }
        // The callback lands on the main looper from the loader's worker;
        // block the test thread only in short slices and idle the looper
        // between them, or the posted callback can never run.
        val deadline = System.currentTimeMillis() + 5_000
        while (!latch.await(50, TimeUnit.MILLISECONDS)) {
            shadowOf(Looper.getMainLooper()).idle()
            if (System.currentTimeMillis() > deadline) {
                break
            }
        }
        assertEquals(0L, latch.count)
        return apply
    }

    @Test
    fun aLaunchWithoutAnIdAppliesNothing() {
        deliver(null)()
        assertTrue(applied.isEmpty())
    }

    @Test
    fun aStagedHandoffAppliesItsExactMap() {
        stagePending(body = "{\"A_NAME\":\"a value with spaces\",\"B\":\"\"}")
        deliver(id)()
        assertEquals(
            linkedMapOf("A_NAME" to "a value with spaces", "B" to ""),
            applied,
        )
    }

    @Test
    fun aClaimFailureSurfacesOnApply() {
        stagePending(body = "{")
        try {
            deliver(id)()
            fail("a malformed handoff fails the apply")
        } catch (expected: IllegalStateException) {
        }
        assertTrue(applied.isEmpty())
    }

    @Test
    fun aDevUrlIsDroppedOutsideADebuggableApp() {
        val flags = context.applicationInfo.flags
        context.applicationInfo.flags = flags and ApplicationInfo.FLAG_DEBUGGABLE.inv()
        try {
            stagePending(
                body = "{\"WATERUI_DEV_URL\":\"http://example.invalid:8080/\",\"OTHER\":\"kept\"}",
            )
            deliver(id)()
        } finally {
            context.applicationInfo.flags = flags
        }
        assertNull(applied["WATERUI_DEV_URL"])
        assertEquals("kept", applied["OTHER"])
    }

    @Test
    fun aDebuggableAppKeepsItsDevUrl() {
        val flags = context.applicationInfo.flags
        context.applicationInfo.flags = flags or ApplicationInfo.FLAG_DEBUGGABLE
        try {
            stagePending(body = "{\"WATERUI_DEV_URL\":\"http://example.invalid:8080/\"}")
            deliver(id)()
        } finally {
            context.applicationInfo.flags = flags
        }
        assertEquals("http://example.invalid:8080/", applied["WATERUI_DEV_URL"])
    }
}
