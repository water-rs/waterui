package dev.waterui.hydrolysis

import android.content.ContentProvider
import android.content.ContentValues
import android.database.Cursor
import android.net.Uri
import android.os.Binder
import android.os.ParcelFileDescriptor
import android.os.Process
import android.system.Os
import android.system.OsConstants
import java.io.File
import java.io.FileNotFoundException
import java.nio.file.Files

/**
 * Creates the write-only descriptor a `content write` lands on, inside the
 * per-id directory the provider already allocated. The seam exists so the
 * atomic create-plus-handoff is exchangeable in unit tests; the production
 * implementation below is the only one the SDK ever installs.
 */
internal fun interface PendingDescriptorFactory {
    fun create(dir: File): ParcelFileDescriptor
}

/**
 * The production [PendingDescriptorFactory]: one atomic
 * `O_WRONLY|O_CREAT|O_EXCL|O_CLOEXEC|O_NOFOLLOW` create at mode 0600 —
 * the returned descriptor duplicates the original fd, which is always
 * closed, so the one-shot file can never be truncated or raced open.
 */
private object OsPendingDescriptorFactory : PendingDescriptorFactory {
    override fun create(dir: File): ParcelFileDescriptor {
        val pending = LaunchEnvironmentStore.pendingFile(dir)
        val fd = try {
            Os.open(
                pending.absolutePath,
                OsConstants.O_WRONLY or OsConstants.O_CREAT or OsConstants.O_EXCL
                    or OsConstants.O_CLOEXEC or OsConstants.O_NOFOLLOW,
                OsConstants.S_IRUSR or OsConstants.S_IWUSR,
            )
        } catch (error: android.system.ErrnoException) {
            throw FileNotFoundException("the launch environment slot is not available")
        }
        try {
            return ParcelFileDescriptor.dup(fd)
        } finally {
            Os.close(fd)
        }
    }
}

/**
 * The launch-environment handoff's device-side endpoint. The CLI feeds an
 * opaque environment payload into a `content write` call against
 * `<package>.waterui.launch-environment/<id>`; the provider allocates the
 * id's private directory once and holds the payload inside it until the
 * activity claims it. `am` arguments carry only the id — values never
 * enter a process table.
 *
 * Only the app's own process and the shell (`com.android.shell`, the only
 * plausible holder of this manifest's `android.permission.DUMP`
 * requirement) may reach the write path; everything else is refused before
 * any file is touched.
 */
class LaunchEnvironmentProvider : ContentProvider {

    private val descriptorFactory: PendingDescriptorFactory

    constructor() {
        descriptorFactory = OsPendingDescriptorFactory
    }

    internal constructor(descriptorFactory: PendingDescriptorFactory) {
        this.descriptorFactory = descriptorFactory
    }

    override fun onCreate(): Boolean = true

    private fun requireAuthorised() {
        val uid = Binder.getCallingUid()
        if (uid == Process.myUid()) return
        val packages = context?.packageManager?.getPackagesForUid(uid).orEmpty()
        if (!packages.contains("com.android.shell")) {
            throw SecurityException("caller is not the application or the shell")
        }
    }

    /**
     * Write-only: `content write` lands the payload's bytes in the pending
     * slot of `id`'s freshly allocated private directory. An id that
     * already owns a directory — pending, active, or anything else —
     * refuses reallocation before a descriptor exists, so the slot is
     * one-shot for the handoff's whole lifetime.
     */
    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        requireAuthorised()
        if (mode != "w") {
            throw FileNotFoundException("the launch environment is write-only")
        }
        val context = checkNotNull(context) { "provider is not attached" }
        val segments = uri.pathSegments
        if (segments.size != 1) {
            throw FileNotFoundException("the launch environment URI names exactly one id")
        }
        val dir = try {
            LaunchEnvironmentStore.allocateId(context, segments[0])
        } catch (error: Exception) {
            throw FileNotFoundException("the launch environment slot is not available")
        }
        return descriptorFactory.create(dir)
    }

    /**
     * Removes exactly the payload files of the id the URI names — pending
     * or active — under its private directory, which stays behind as a
     * tombstone so the id can never be reallocated. Used for the rollback
     * of a launch that failed after staging; the directory is never
     * deleted here.
     */
    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int {
        requireAuthorised()
        if (selection != null || !selectionArgs.isNullOrEmpty()) {
            throw IllegalArgumentException("the launch environment takes no selection")
        }
        val context = checkNotNull(context) { "provider is not attached" }
        val segments = uri.pathSegments
        if (segments.size != 1) {
            throw IllegalArgumentException("the launch environment URI names exactly one id")
        }
        val dir = try {
            LaunchEnvironmentStore.deletableIdDirectory(context, segments[0])
        } catch (error: Exception) {
            throw IllegalArgumentException("the launch environment URI names exactly one id")
        }
        if (dir == null) {
            return 0
        }
        var removed = 0
        for (file in listOf(
            LaunchEnvironmentStore.pendingFile(dir),
            LaunchEnvironmentStore.activeFile(dir),
        )) {
            if (Files.deleteIfExists(file.toPath())) {
                removed += 1
            }
        }
        return removed
    }

    override fun query(
        uri: Uri,
        projection: Array<out String>?,
        selection: String?,
        selectionArgs: Array<out String>?,
        sortOrder: String?,
    ): Cursor = throw UnsupportedOperationException("the launch environment is write-only")

    override fun insert(uri: Uri, values: ContentValues?): Uri =
        throw UnsupportedOperationException("the launch environment is write-only")

    override fun update(
        uri: Uri,
        values: ContentValues?,
        selection: String?,
        selectionArgs: Array<out String>?,
    ): Int = throw UnsupportedOperationException("the launch environment is write-only")

    override fun getType(uri: Uri): String =
        throw UnsupportedOperationException("the launch environment is write-only")

    override fun call(method: String, arg: String?, extras: android.os.Bundle?): android.os.Bundle =
        throw UnsupportedOperationException("the launch environment is write-only")
}
