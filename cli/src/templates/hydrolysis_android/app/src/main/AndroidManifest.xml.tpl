<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android">

{% for permission in ctx.android_permissions %}
    <uses-permission android:name="{{ permission }}" />
{% endfor %}

    <application
        android:allowBackup="true"
        android:enableOnBackInvokedCallback="true"
        android:icon="@mipmap/ic_launcher"
        android:label="@string/app_name"
        android:resizeableActivity="true"
        android:roundIcon="@mipmap/ic_launcher_round"
        android:supportsRtl="true"
        android:theme="@style/Theme.WaterUIApp">
        <activity
            android:name=".MainActivity"
            android:configChanges="screenSize|smallestScreenSize|screenLayout|orientation|keyboardHidden"
            android:exported="true"
            android:windowSoftInputMode="adjustNothing"
            android:launchMode="singleTask"
            android:theme="@style/Theme.WaterUIApp.Launch">
            <intent-filter>
                <action android:name="android.intent.action.MAIN" />
                <category android:name="android.intent.category.LAUNCHER" />
            </intent-filter>
        </activity>
        <!-- begin waterui android manifest components -->
        <!-- end waterui android manifest components -->
    </application>

</manifest>
