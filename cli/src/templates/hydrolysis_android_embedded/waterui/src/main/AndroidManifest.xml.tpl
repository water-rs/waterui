<manifest xmlns:android="http://schemas.android.com/apk/res/android">
{% for permission in ctx.android_permissions %}
    <uses-permission android:name="{{ permission }}" />
{% endfor %}
    <application>
        <provider
            android:name="dev.waterui.hydrolysis.LaunchEnvironmentProvider"
            android:authorities="${applicationId}.waterui.launch-environment"
            android:exported="true"
            android:permission="android.permission.DUMP"
            android:grantUriPermissions="false" />
        <!-- begin waterui android manifest components -->
        <!-- end waterui android manifest components -->
    </application>
</manifest>
