package dev.waterui.hydrolysis

import android.content.Context
import android.util.JsonReader
import android.util.JsonToken
import java.io.File
import java.io.InputStreamReader
import java.nio.CharBuffer
import java.nio.charset.CharacterCodingException
import java.nio.charset.CodingErrorAction
import java.nio.charset.StandardCharsets
import java.nio.file.FileAlreadyExistsException
import java.nio.file.Files
import java.nio.file.LinkOption
import java.nio.file.NoSuchFileException
import java.nio.file.StandardCopyOption
import java.nio.file.attribute.BasicFileAttributes
import java.nio.file.attribute.PosixFilePermission
import java.nio.file.attribute.PosixFilePermissions

/**
 * The app-private store a launch-environment handoff lands in. Each id owns
 * exactly one private directory — `launch-environments/<id>` — created once
 * with owner-only birth permissions and never reusable: an existing per-id
 * directory (any file type, symlink included) refuses reallocation, which
 * is what makes the handoff one-shot. The payload lives inside it as
 * `pending` until the activity's claim moves it to `active`, where it
 * stays while Android keeps the launch around — a later process or task
 * recreation carrying the same id reads the ACTIVE file again, the
 * expected state, not a fallback.
 */
internal object LaunchEnvironmentStore {

    private val OWNER_ONLY_DIR = setOf(
        PosixFilePermission.OWNER_READ,
        PosixFilePermission.OWNER_WRITE,
        PosixFilePermission.OWNER_EXECUTE,
    )

    private val ENVIRONMENT_ID =
        Regex("^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$")

    private fun validateId(id: String): String {
        require(ENVIRONMENT_ID.matches(id)) { "the launch-environment id is invalid" }
        return id
    }

    /** The private namespace — a path, never a filesystem operation. */
    fun root(context: Context): File =
        File(context.noBackupFilesDir, "launch-environments")

    /** The per-id directory — a path, never a filesystem operation.
     *  The id is validated before the context is even asked for a
     *  directory: an invalid id must not touch the filesystem. */
    fun idDirectory(context: Context, id: String): File {
        val validated = validateId(id)
        return File(root(context), validated)
    }

    /**
     * `path` must be a genuinely private directory: a real directory and
     * owner-only bits, inspected without following links. A symlink, a
     * file, or a directory that leaks group/other access fails the store —
     * nothing is chmodded into place after the fact.
     */
    private fun requirePrivateDirectory(path: java.nio.file.Path) {
        val attributes = Files.readAttributes(
            path,
            BasicFileAttributes::class.java,
            LinkOption.NOFOLLOW_LINKS,
        )
        val permissions =
            Files.getPosixFilePermissions(path, LinkOption.NOFOLLOW_LINKS)
        if (!attributes.isDirectory ||
            permissions.any {
                it.name.startsWith("GROUP_") || it.name.startsWith("OTHERS_")
            }
        ) {
            throw IllegalStateException(
                "the launch environment namespace is not a private directory",
            )
        }
    }

    /**
     * The namespace, created once with owner-only birth permissions — or
     * validated as genuinely private when it already exists. An existing
     * root that is a symlink, a file, or not private refuses the store
     * rather than being followed or repaired.
     */
    private fun ensureRoot(context: Context) {
        val path = root(context).toPath()
        if (!Files.exists(path, LinkOption.NOFOLLOW_LINKS)) {
            try {
                Files.createDirectory(
                    path,
                    PosixFilePermissions.asFileAttribute(OWNER_ONLY_DIR),
                )
            } catch (error: FileAlreadyExistsException) {
                // A concurrent allocation won — the winner's entry is
                // validated below like any pre-existing root.
            }
        }
        requirePrivateDirectory(path)
    }

    /**
     * The id's private directory when it genuinely exists — `null` when
     * root or the id entry is absent, a failure when either exists but is
     * not a private directory. Readers and deleters validate this way and
     * never create.
     */
    private fun existingIdDirectory(context: Context, id: String): File? {
        val validated = validateId(id)
        val root = root(context).toPath()
        if (!Files.exists(root, LinkOption.NOFOLLOW_LINKS)) {
            return null
        }
        requirePrivateDirectory(root)
        val dir = File(root.toFile(), validated).toPath()
        if (!Files.exists(dir, LinkOption.NOFOLLOW_LINKS)) {
            return null
        }
        requirePrivateDirectory(dir)
        return dir.toFile()
    }

    fun pendingFile(dir: File): File = File(dir, "pending")

    fun activeFile(dir: File): File = File(dir, "active")

    /**
     * Allocate `id`'s private directory — once for the handoff's lifetime.
     * An existing entry at the path (pending, active, or a foreign file
     * type such as a symlink) refuses reallocation before any payload
     * descriptor is created. The namespace directory is born owner-only;
     * nothing chmods it afterwards.
     */
    fun allocateId(context: Context, id: String): File {
        val validated = validateId(id)
        ensureRoot(context)
        val dir = File(root(context), validated)
        if (Files.exists(dir.toPath(), LinkOption.NOFOLLOW_LINKS)) {
            throw FileAlreadyExistsException(dir.absolutePath)
        }
        Files.createDirectory(
            dir.toPath(),
            PosixFilePermissions.asFileAttribute(OWNER_ONLY_DIR),
        )
        return dir
    }

    /**
     * The id's private directory for a delete — `null` when nothing was
     * ever allocated under it, so an unknown id removes nothing and
     * creates nothing.
     */
    fun deletableIdDirectory(context: Context, id: String): File? =
        existingIdDirectory(context, id)

    /**
     * Read `file` only if it is genuinely there: a symlink fails the open,
     * and the private regular-file invariants (regular, owner-only) are
     * verified without following links.
     */
    private fun readRegular(file: File): ByteArray {
        val path = file.toPath()
        val attributes = Files.readAttributes(
            path,
            BasicFileAttributes::class.java,
            LinkOption.NOFOLLOW_LINKS,
        )
        val permissions = Files.getPosixFilePermissions(path, LinkOption.NOFOLLOW_LINKS)
        if (!attributes.isRegularFile ||
            permissions.any {
                it.name.startsWith("GROUP_") || it.name.startsWith("OTHERS_")
            }
        ) {
            throw IllegalStateException("the launch environment is not a private regular file")
        }
        Files.newInputStream(path, LinkOption.NOFOLLOW_LINKS).use { stream ->
            return stream.readBytes()
        }
    }

    private fun readIfActive(active: File): LaunchEnvironmentValues? {
        if (!Files.exists(active.toPath(), LinkOption.NOFOLLOW_LINKS)) {
            return null
        }
        return LaunchEnvironmentValues.parse(readRegular(active))
    }

    /**
     * Read the handoff `id` names. An ACTIVE config answers first — and if
     * a pending file sits alongside it, the state is corrupt and fails. With
     * no active file, the pending payload parses then moves into place
     * atomically; a `NoSuchFileException` from a pending read or the move
     * means a concurrent claim just won, so the same id's now-ACTIVE file
     * answers — the expected transition, not a retry. [afterPendingRead] is
     * a per-call seam the two-reader claim test drives deterministically.
     */
    fun claim(
        context: Context,
        id: String,
        afterPendingRead: () -> Unit = {},
    ): LaunchEnvironmentValues {
        val dir = existingIdDirectory(context, id)
            ?: throw IllegalStateException("no launch environment under the given id")
        val pending = pendingFile(dir)
        val active = activeFile(dir)
        readIfActive(active)?.let { values ->
            if (Files.exists(pending.toPath(), LinkOption.NOFOLLOW_LINKS)) {
                throw IllegalStateException("conflicting launch-environment state")
            }
            return values
        }
        val parsed = try {
            LaunchEnvironmentValues.parse(readRegular(pending))
        } catch (error: NoSuchFileException) {
            val claimed = readIfActive(active)
                ?: throw IllegalStateException("no launch environment under the given id")
            return claimed
        }
        afterPendingRead()
        try {
            Files.move(pending.toPath(), active.toPath(), StandardCopyOption.ATOMIC_MOVE)
        } catch (error: NoSuchFileException) {
            val claimed = readIfActive(active)
                ?: throw IllegalStateException("conflicting launch-environment state")
            return claimed
        } catch (error: FileAlreadyExistsException) {
            val claimed = readIfActive(active)
                ?: throw IllegalStateException("conflicting launch-environment state")
            return claimed
        }
        return parsed
    }
}

/**
 * The immutable map a claimed handoff decoded into. The raw map stays
 * private — `toString` and any error surface keys only, never values.
 */
internal class LaunchEnvironmentValues private constructor(
    private val values: Map<String, String>,
) {

    /**
     * Apply every entry through [installer]. `WATERUI_DEV_URL` reaches a
     * debuggable app only — a release build always renders its staged
     * bundle. An installer failure names the validated key, never the
     * value or the cause.
     */
    fun applyTo(debuggable: Boolean, installer: (String, String) -> Unit) {
        for ((key, value) in values) {
            if (key == "WATERUI_DEV_URL" && !debuggable) {
                continue
            }
            try {
                installer(key, value)
            } catch (error: Exception) {
                throw IllegalStateException(
                    "could not apply launch environment variable $key",
                )
            }
        }
    }

    override fun toString(): String =
        "LaunchEnvironmentValues(keys=${values.keys.sorted()})"

    internal companion object {

        private val utf8Decoder
            get() = StandardCharsets.UTF_8.newDecoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)

        private fun validateUnicode(text: String) {
            val encoder = StandardCharsets.UTF_8.newEncoder()
                .onMalformedInput(CodingErrorAction.REPORT)
                .onUnmappableCharacter(CodingErrorAction.REPORT)
            try {
                encoder.encode(CharBuffer.wrap(text))
            } catch (error: CharacterCodingException) {
                throw IllegalStateException(
                    "the launch environment holds malformed text",
                )
            }
        }

        private fun invalid(): Nothing =
            throw IllegalStateException("the launch environment is not a valid JSON object")

        /**
         * Strictly decode the payload: exact UTF-8, exactly one JSON object
         * (outer whitespace allowed), nonempty `=`/`NUL`-free unique string
         * keys, string-only `NUL`-free values. Every failure is the fixed
         * message — platform parser errors can echo the body, so causes
         * are dropped.
         */
        fun parse(payload: ByteArray): LaunchEnvironmentValues {
            try {
                JsonReader(
                    InputStreamReader(payload.inputStream(), utf8Decoder),
                ).use { reader ->
                    reader.isLenient = false
                    val values = LinkedHashMap<String, String>()
                    reader.beginObject()
                    while (reader.hasNext()) {
                        val name = reader.nextName()
                        if (name.isEmpty() ||
                            name.contains('=') ||
                            name.contains('\u0000') ||
                            values.containsKey(name)
                        ) {
                            invalid()
                        }
                        if (reader.peek() != JsonToken.STRING) {
                            invalid()
                        }
                        val value = reader.nextString()
                        if (value.contains('\u0000')) {
                            invalid()
                        }
                        validateUnicode(name)
                        validateUnicode(value)
                        values[name] = value
                    }
                    reader.endObject()
                    if (reader.peek() != JsonToken.END_DOCUMENT) {
                        invalid()
                    }
                    return LaunchEnvironmentValues(values)
                }
            } catch (error: Exception) {
                // Every failure — platform parser ISEs included — is the
                // same fixed message with no cause: SDK errors can echo
                // the payload body.
                invalid()
            }
        }
    }
}
