<manifest xmlns:android="http://schemas.android.com/apk/res/android">
{% for permission in ctx.android_permissions %}
    <uses-permission android:name="android.permission.{{ permission.name }}" />
{% endfor %}
</manifest>
