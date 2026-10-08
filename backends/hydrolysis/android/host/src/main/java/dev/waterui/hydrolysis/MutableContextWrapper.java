// Copyright (c) The WaterUI Contributors — see NOTICE.md
package dev.waterui.hydrolysis;

import android.annotation.SuppressLint;

/**
 * A {@link android.content.Context} whose base changes after construction —
 * the platform's {@code android.content.MutableContextWrapper} idea, written
 * for the API 31 floor the host builds against (the framework class is
 * API-35+). Every non-final call delegates to {@link #getBaseContext()}; the
 * final convenience methods ({@code getString}, {@code getColor},
 * {@code obtainStyledAttributes}, ...) route through those delegates.
 *
 * <p>The Hydrolysis session retains platform-view instances across host
 * rebinding; a {@link android.view.View} created against one Activity must
 * follow the next host's context without being rebuilt (a system
 * {@code WebView} would otherwise leak the first Activity and lose its
 * configuration).
 */
// The methods below only forward — the permission and API-level requirements
// the lint checks associate with each `Context` call belong to the delegate's
// caller (the `View` holding this wrapper), which invokes the same API with the
// same context as without the wrapper.
@SuppressLint({"NewApi", "MissingPermission", "UnspecifiedRegisterReceiverFlag"})
public class MutableContextWrapper extends android.content.Context {
    private android.content.Context baseContext;

    public MutableContextWrapper(android.content.Context base) {
        this.baseContext = base;
    }

    /** Rebind to the new host's context. */
    public void setBaseContext(android.content.Context base) {
        this.baseContext = base;
    }

    public android.content.Context getBaseContext() {
        return baseContext;
    }

    @Override public boolean bindIsolatedService(android.content.Intent arg0, android.content.Context.BindServiceFlags arg1, java.lang.String arg2, java.util.concurrent.Executor arg3, android.content.ServiceConnection arg4) { return baseContext.bindIsolatedService(arg0, arg1, arg2, arg3, arg4); }
    @Override public boolean bindIsolatedService(android.content.Intent arg0, int arg1, java.lang.String arg2, java.util.concurrent.Executor arg3, android.content.ServiceConnection arg4) { return baseContext.bindIsolatedService(arg0, arg1, arg2, arg3, arg4); }
    @Override public boolean bindService(android.content.Intent arg0, android.content.Context.BindServiceFlags arg1, java.util.concurrent.Executor arg2, android.content.ServiceConnection arg3) { return baseContext.bindService(arg0, arg1, arg2, arg3); }
    @Override public boolean bindService(android.content.Intent arg0, android.content.ServiceConnection arg1, android.content.Context.BindServiceFlags arg2) { return baseContext.bindService(arg0, arg1, arg2); }
    @Override public boolean bindService(android.content.Intent arg0, android.content.ServiceConnection arg1, int arg2) { return baseContext.bindService(arg0, arg1, arg2); }
    @Override public boolean bindService(android.content.Intent arg0, int arg1, java.util.concurrent.Executor arg2, android.content.ServiceConnection arg3) { return baseContext.bindService(arg0, arg1, arg2, arg3); }
    @Override public boolean bindServiceAsUser(android.content.Intent arg0, android.content.ServiceConnection arg1, android.content.Context.BindServiceFlags arg2, android.os.UserHandle arg3) { return baseContext.bindServiceAsUser(arg0, arg1, arg2, arg3); }
    @Override public boolean bindServiceAsUser(android.content.Intent arg0, android.content.ServiceConnection arg1, int arg2, android.os.UserHandle arg3) { return baseContext.bindServiceAsUser(arg0, arg1, arg2, arg3); }
    @Override public int checkCallingOrSelfPermission(java.lang.String arg0) { return baseContext.checkCallingOrSelfPermission(arg0); }
    @Override public int checkCallingOrSelfUriPermission(android.net.Uri arg0, int arg1) { return baseContext.checkCallingOrSelfUriPermission(arg0, arg1); }
    @Override public int[] checkCallingOrSelfUriPermissions(java.util.List<android.net.Uri> arg0, int arg1) { return baseContext.checkCallingOrSelfUriPermissions(arg0, arg1); }
    @Override public int checkCallingPermission(java.lang.String arg0) { return baseContext.checkCallingPermission(arg0); }
    @Override public int checkCallingUriPermission(android.net.Uri arg0, int arg1) { return baseContext.checkCallingUriPermission(arg0, arg1); }
    @Override public int[] checkCallingUriPermissions(java.util.List<android.net.Uri> arg0, int arg1) { return baseContext.checkCallingUriPermissions(arg0, arg1); }
    @Override public int checkContentUriPermissionFull(android.net.Uri arg0, int arg1, int arg2, int arg3) { return baseContext.checkContentUriPermissionFull(arg0, arg1, arg2, arg3); }
    @Override public int checkPermission(java.lang.String arg0, int arg1, int arg2) { return baseContext.checkPermission(arg0, arg1, arg2); }
    @Override public int checkSelfPermission(java.lang.String arg0) { return baseContext.checkSelfPermission(arg0); }
    @Override public int checkUriPermission(android.net.Uri arg0, int arg1, int arg2, int arg3) { return baseContext.checkUriPermission(arg0, arg1, arg2, arg3); }
    @Override public int checkUriPermission(android.net.Uri arg0, java.lang.String arg1, java.lang.String arg2, int arg3, int arg4, int arg5) { return baseContext.checkUriPermission(arg0, arg1, arg2, arg3, arg4, arg5); }
    @Override public int[] checkUriPermissions(java.util.List<android.net.Uri> arg0, int arg1, int arg2, int arg3) { return baseContext.checkUriPermissions(arg0, arg1, arg2, arg3); }
    @Override public void clearWallpaper() throws java.io.IOException { baseContext.clearWallpaper(); }
    @Override public android.content.Context createAttributionContext(java.lang.String arg0) { return baseContext.createAttributionContext(arg0); }
    @Override public android.content.Context createConfigurationContext(android.content.res.Configuration arg0) { return baseContext.createConfigurationContext(arg0); }
    @Override public android.content.Context createContext(android.content.ContextParams arg0) { return baseContext.createContext(arg0); }
    @Override public android.content.Context createContextForSplit(java.lang.String arg0) throws android.content.pm.PackageManager.NameNotFoundException { return baseContext.createContextForSplit(arg0); }
    @Override public android.content.Context createDeviceContext(int arg0) { return baseContext.createDeviceContext(arg0); }
    @Override public android.content.Context createDeviceProtectedStorageContext() { return baseContext.createDeviceProtectedStorageContext(); }
    @Override public android.content.Context createDisplayContext(android.view.Display arg0) { return baseContext.createDisplayContext(arg0); }
    @Override public android.content.Context createPackageContext(java.lang.String arg0, int arg1) throws android.content.pm.PackageManager.NameNotFoundException { return baseContext.createPackageContext(arg0, arg1); }
    @Override public android.content.Context createWindowContext(android.view.Display arg0, int arg1, android.os.Bundle arg2) { return baseContext.createWindowContext(arg0, arg1, arg2); }
    @Override public android.content.Context createWindowContext(int arg0, android.os.Bundle arg1) { return baseContext.createWindowContext(arg0, arg1); }
    @Override public java.lang.String[] databaseList() { return baseContext.databaseList(); }
    @Override public boolean deleteDatabase(java.lang.String arg0) { return baseContext.deleteDatabase(arg0); }
    @Override public boolean deleteFile(java.lang.String arg0) { return baseContext.deleteFile(arg0); }
    @Override public boolean deleteSharedPreferences(java.lang.String arg0) { return baseContext.deleteSharedPreferences(arg0); }
    @Override public void enforceCallingOrSelfPermission(java.lang.String arg0, java.lang.String arg1) { baseContext.enforceCallingOrSelfPermission(arg0, arg1); }
    @Override public void enforceCallingOrSelfUriPermission(android.net.Uri arg0, int arg1, java.lang.String arg2) { baseContext.enforceCallingOrSelfUriPermission(arg0, arg1, arg2); }
    @Override public void enforceCallingPermission(java.lang.String arg0, java.lang.String arg1) { baseContext.enforceCallingPermission(arg0, arg1); }
    @Override public void enforceCallingUriPermission(android.net.Uri arg0, int arg1, java.lang.String arg2) { baseContext.enforceCallingUriPermission(arg0, arg1, arg2); }
    @Override public void enforcePermission(java.lang.String arg0, int arg1, int arg2, java.lang.String arg3) { baseContext.enforcePermission(arg0, arg1, arg2, arg3); }
    @Override public void enforceUriPermission(android.net.Uri arg0, int arg1, int arg2, int arg3, java.lang.String arg4) { baseContext.enforceUriPermission(arg0, arg1, arg2, arg3, arg4); }
    @Override public void enforceUriPermission(android.net.Uri arg0, java.lang.String arg1, java.lang.String arg2, int arg3, int arg4, int arg5, java.lang.String arg6) { baseContext.enforceUriPermission(arg0, arg1, arg2, arg3, arg4, arg5, arg6); }
    @Override public java.lang.String[] fileList() { return baseContext.fileList(); }
    @Override public android.content.Context getApplicationContext() { return baseContext.getApplicationContext(); }
    @Override public android.content.pm.ApplicationInfo getApplicationInfo() { return baseContext.getApplicationInfo(); }
    @Override public android.content.res.AssetManager getAssets() { return baseContext.getAssets(); }
    @Override public android.content.AttributionSource getAttributionSource() { return baseContext.getAttributionSource(); }
    @Override public java.lang.String getAttributionTag() { return baseContext.getAttributionTag(); }
    @Override public java.io.File getCacheDir() { return baseContext.getCacheDir(); }
    @Override public java.lang.ClassLoader getClassLoader() { return baseContext.getClassLoader(); }
    @Override public java.io.File getCodeCacheDir() { return baseContext.getCodeCacheDir(); }
    @Override public android.content.ContentResolver getContentResolver() { return baseContext.getContentResolver(); }
    @Override public java.io.File getDataDir() { return baseContext.getDataDir(); }
    @Override public java.io.File getDatabasePath(java.lang.String arg0) { return baseContext.getDatabasePath(arg0); }
    @Override public int getDeviceId() { return baseContext.getDeviceId(); }
    @Override public java.io.File getDir(java.lang.String arg0, int arg1) { return baseContext.getDir(arg0, arg1); }
    @Override public android.view.Display getDisplay() { return baseContext.getDisplay(); }
    @Override public java.io.File getExternalCacheDir() { return baseContext.getExternalCacheDir(); }
    @Override public java.io.File[] getExternalCacheDirs() { return baseContext.getExternalCacheDirs(); }
    @Override public java.io.File getExternalFilesDir(java.lang.String arg0) { return baseContext.getExternalFilesDir(arg0); }
    @Override public java.io.File[] getExternalFilesDirs(java.lang.String arg0) { return baseContext.getExternalFilesDirs(arg0); }
    @Override public java.io.File[] getExternalMediaDirs() { return baseContext.getExternalMediaDirs(); }
    @Override public java.io.File getFileStreamPath(java.lang.String arg0) { return baseContext.getFileStreamPath(arg0); }
    @Override public java.io.File getFilesDir() { return baseContext.getFilesDir(); }
    @Override public java.util.concurrent.Executor getMainExecutor() { return baseContext.getMainExecutor(); }
    @Override public android.os.Looper getMainLooper() { return baseContext.getMainLooper(); }
    @Override public java.io.File getNoBackupFilesDir() { return baseContext.getNoBackupFilesDir(); }
    @Override public java.io.File getObbDir() { return baseContext.getObbDir(); }
    @Override public java.io.File[] getObbDirs() { return baseContext.getObbDirs(); }
    @Override public java.lang.String getOpPackageName() { return baseContext.getOpPackageName(); }
    @Override public java.lang.String getPackageCodePath() { return baseContext.getPackageCodePath(); }
    @Override public android.content.pm.PackageManager getPackageManager() { return baseContext.getPackageManager(); }
    @Override public java.lang.String getPackageName() { return baseContext.getPackageName(); }
    @Override public java.lang.String getPackageResourcePath() { return baseContext.getPackageResourcePath(); }
    @Override public android.content.ContextParams getParams() { return baseContext.getParams(); }
    @Override public android.content.res.Resources getResources() { return baseContext.getResources(); }
    @Override public android.content.SharedPreferences getSharedPreferences(java.lang.String arg0, int arg1) { return baseContext.getSharedPreferences(arg0, arg1); }
    @Override public java.lang.Object getSystemService(java.lang.String arg0) { return baseContext.getSystemService(arg0); }
    @Override public java.lang.String getSystemServiceName(java.lang.Class<?> arg0) { return baseContext.getSystemServiceName(arg0); }
    @Override public android.content.res.Resources.Theme getTheme() { return baseContext.getTheme(); }
    @Override public android.graphics.drawable.Drawable getWallpaper() { return baseContext.getWallpaper(); }
    @Override public int getWallpaperDesiredMinimumHeight() { return baseContext.getWallpaperDesiredMinimumHeight(); }
    @Override public int getWallpaperDesiredMinimumWidth() { return baseContext.getWallpaperDesiredMinimumWidth(); }
    @Override public void grantUriPermission(java.lang.String arg0, android.net.Uri arg1, int arg2) { baseContext.grantUriPermission(arg0, arg1, arg2); }
    @Override public boolean isDeviceProtectedStorage() { return baseContext.isDeviceProtectedStorage(); }
    @Override public boolean isRestricted() { return baseContext.isRestricted(); }
    @Override public boolean isUiContext() { return baseContext.isUiContext(); }
    @Override public boolean moveDatabaseFrom(android.content.Context arg0, java.lang.String arg1) { return baseContext.moveDatabaseFrom(arg0, arg1); }
    @Override public boolean moveSharedPreferencesFrom(android.content.Context arg0, java.lang.String arg1) { return baseContext.moveSharedPreferencesFrom(arg0, arg1); }
    @Override public java.io.FileInputStream openFileInput(java.lang.String arg0) throws java.io.FileNotFoundException { return baseContext.openFileInput(arg0); }
    @Override public java.io.FileOutputStream openFileOutput(java.lang.String arg0, int arg1) throws java.io.FileNotFoundException { return baseContext.openFileOutput(arg0, arg1); }
    @Override public android.database.sqlite.SQLiteDatabase openOrCreateDatabase(java.lang.String arg0, int arg1, android.database.sqlite.SQLiteDatabase.CursorFactory arg2) { return baseContext.openOrCreateDatabase(arg0, arg1, arg2); }
    @Override public android.database.sqlite.SQLiteDatabase openOrCreateDatabase(java.lang.String arg0, int arg1, android.database.sqlite.SQLiteDatabase.CursorFactory arg2, android.database.DatabaseErrorHandler arg3) { return baseContext.openOrCreateDatabase(arg0, arg1, arg2, arg3); }
    @Override public android.graphics.drawable.Drawable peekWallpaper() { return baseContext.peekWallpaper(); }
    @Override public void registerComponentCallbacks(android.content.ComponentCallbacks arg0) { baseContext.registerComponentCallbacks(arg0); }
    @Override public void registerDeviceIdChangeListener(java.util.concurrent.Executor arg0, java.util.function.IntConsumer arg1) { baseContext.registerDeviceIdChangeListener(arg0, arg1); }
    @Override public android.content.Intent registerReceiver(android.content.BroadcastReceiver arg0, android.content.IntentFilter arg1) { return baseContext.registerReceiver(arg0, arg1); }
    @Override public android.content.Intent registerReceiver(android.content.BroadcastReceiver arg0, android.content.IntentFilter arg1, int arg2) { return baseContext.registerReceiver(arg0, arg1, arg2); }
    @Override public android.content.Intent registerReceiver(android.content.BroadcastReceiver arg0, android.content.IntentFilter arg1, java.lang.String arg2, android.os.Handler arg3) { return baseContext.registerReceiver(arg0, arg1, arg2, arg3); }
    @Override public android.content.Intent registerReceiver(android.content.BroadcastReceiver arg0, android.content.IntentFilter arg1, java.lang.String arg2, android.os.Handler arg3, int arg4) { return baseContext.registerReceiver(arg0, arg1, arg2, arg3, arg4); }
    @Override public void removeStickyBroadcast(android.content.Intent arg0) { baseContext.removeStickyBroadcast(arg0); }
    @Override public void removeStickyBroadcastAsUser(android.content.Intent arg0, android.os.UserHandle arg1) { baseContext.removeStickyBroadcastAsUser(arg0, arg1); }
    @Override public void revokeSelfPermissionOnKill(java.lang.String arg0) { baseContext.revokeSelfPermissionOnKill(arg0); }
    @Override public void revokeSelfPermissionsOnKill(java.util.Collection<java.lang.String> arg0) { baseContext.revokeSelfPermissionsOnKill(arg0); }
    @Override public void revokeUriPermission(android.net.Uri arg0, int arg1) { baseContext.revokeUriPermission(arg0, arg1); }
    @Override public void revokeUriPermission(java.lang.String arg0, android.net.Uri arg1, int arg2) { baseContext.revokeUriPermission(arg0, arg1, arg2); }
    @Override public void sendBroadcast(android.content.Intent arg0) { baseContext.sendBroadcast(arg0); }
    @Override public void sendBroadcast(android.content.Intent arg0, java.lang.String arg1) { baseContext.sendBroadcast(arg0, arg1); }
    @Override public void sendBroadcast(android.content.Intent arg0, java.lang.String arg1, android.os.Bundle arg2) { baseContext.sendBroadcast(arg0, arg1, arg2); }
    @Override public void sendBroadcastAsUser(android.content.Intent arg0, android.os.UserHandle arg1) { baseContext.sendBroadcastAsUser(arg0, arg1); }
    @Override public void sendBroadcastAsUser(android.content.Intent arg0, android.os.UserHandle arg1, java.lang.String arg2) { baseContext.sendBroadcastAsUser(arg0, arg1, arg2); }
    @Override public void sendBroadcastWithMultiplePermissions(android.content.Intent arg0, java.lang.String[] arg1) { baseContext.sendBroadcastWithMultiplePermissions(arg0, arg1); }
    @Override public void sendOrderedBroadcast(android.content.Intent arg0, java.lang.String arg1) { baseContext.sendOrderedBroadcast(arg0, arg1); }
    @Override public void sendOrderedBroadcast(android.content.Intent arg0, java.lang.String arg1, android.content.BroadcastReceiver arg2, android.os.Handler arg3, int arg4, java.lang.String arg5, android.os.Bundle arg6) { baseContext.sendOrderedBroadcast(arg0, arg1, arg2, arg3, arg4, arg5, arg6); }
    @Override public void sendOrderedBroadcast(android.content.Intent arg0, java.lang.String arg1, android.os.Bundle arg2) { baseContext.sendOrderedBroadcast(arg0, arg1, arg2); }
    @Override public void sendOrderedBroadcast(android.content.Intent arg0, java.lang.String arg1, android.os.Bundle arg2, android.content.BroadcastReceiver arg3, android.os.Handler arg4, int arg5, java.lang.String arg6, android.os.Bundle arg7) { baseContext.sendOrderedBroadcast(arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7); }
    @Override public void sendOrderedBroadcast(android.content.Intent arg0, java.lang.String arg1, java.lang.String arg2, android.content.BroadcastReceiver arg3, android.os.Handler arg4, int arg5, java.lang.String arg6, android.os.Bundle arg7) { baseContext.sendOrderedBroadcast(arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7); }
    @Override public void sendOrderedBroadcastAsUser(android.content.Intent arg0, android.os.UserHandle arg1, java.lang.String arg2, android.content.BroadcastReceiver arg3, android.os.Handler arg4, int arg5, java.lang.String arg6, android.os.Bundle arg7) { baseContext.sendOrderedBroadcastAsUser(arg0, arg1, arg2, arg3, arg4, arg5, arg6, arg7); }
    @Override public void sendStickyBroadcast(android.content.Intent arg0) { baseContext.sendStickyBroadcast(arg0); }
    @Override public void sendStickyBroadcast(android.content.Intent arg0, android.os.Bundle arg1) { baseContext.sendStickyBroadcast(arg0, arg1); }
    @Override public void sendStickyBroadcastAsUser(android.content.Intent arg0, android.os.UserHandle arg1) { baseContext.sendStickyBroadcastAsUser(arg0, arg1); }
    @Override public void sendStickyOrderedBroadcast(android.content.Intent arg0, android.content.BroadcastReceiver arg1, android.os.Handler arg2, int arg3, java.lang.String arg4, android.os.Bundle arg5) { baseContext.sendStickyOrderedBroadcast(arg0, arg1, arg2, arg3, arg4, arg5); }
    @Override public void sendStickyOrderedBroadcastAsUser(android.content.Intent arg0, android.os.UserHandle arg1, android.content.BroadcastReceiver arg2, android.os.Handler arg3, int arg4, java.lang.String arg5, android.os.Bundle arg6) { baseContext.sendStickyOrderedBroadcastAsUser(arg0, arg1, arg2, arg3, arg4, arg5, arg6); }
    @Override public void setTheme(int arg0) { baseContext.setTheme(arg0); }
    @Override public void setWallpaper(android.graphics.Bitmap arg0) throws java.io.IOException { baseContext.setWallpaper(arg0); }
    @Override public void setWallpaper(java.io.InputStream arg0) throws java.io.IOException { baseContext.setWallpaper(arg0); }
    @Override public void startActivities(android.content.Intent[] arg0) { baseContext.startActivities(arg0); }
    @Override public void startActivities(android.content.Intent[] arg0, android.os.Bundle arg1) { baseContext.startActivities(arg0, arg1); }
    @Override public void startActivity(android.content.Intent arg0) { baseContext.startActivity(arg0); }
    @Override public void startActivity(android.content.Intent arg0, android.os.Bundle arg1) { baseContext.startActivity(arg0, arg1); }
    @Override public android.content.ComponentName startForegroundService(android.content.Intent arg0) { return baseContext.startForegroundService(arg0); }
    @Override public boolean startInstrumentation(android.content.ComponentName arg0, java.lang.String arg1, android.os.Bundle arg2) { return baseContext.startInstrumentation(arg0, arg1, arg2); }
    @Override public void startIntentSender(android.content.IntentSender arg0, android.content.Intent arg1, int arg2, int arg3, int arg4) throws android.content.IntentSender.SendIntentException { baseContext.startIntentSender(arg0, arg1, arg2, arg3, arg4); }
    @Override public void startIntentSender(android.content.IntentSender arg0, android.content.Intent arg1, int arg2, int arg3, int arg4, android.os.Bundle arg5) throws android.content.IntentSender.SendIntentException { baseContext.startIntentSender(arg0, arg1, arg2, arg3, arg4, arg5); }
    @Override public android.content.ComponentName startService(android.content.Intent arg0) { return baseContext.startService(arg0); }
    @Override public boolean stopService(android.content.Intent arg0) { return baseContext.stopService(arg0); }
    @Override public void unbindService(android.content.ServiceConnection arg0) { baseContext.unbindService(arg0); }
    @Override public void unregisterComponentCallbacks(android.content.ComponentCallbacks arg0) { baseContext.unregisterComponentCallbacks(arg0); }
    @Override public void unregisterDeviceIdChangeListener(java.util.function.IntConsumer arg0) { baseContext.unregisterDeviceIdChangeListener(arg0); }
    @Override public void unregisterReceiver(android.content.BroadcastReceiver arg0) { baseContext.unregisterReceiver(arg0); }
    @Override public void updateServiceGroup(android.content.ServiceConnection arg0, int arg1, int arg2) { baseContext.updateServiceGroup(arg0, arg1, arg2); }
}
