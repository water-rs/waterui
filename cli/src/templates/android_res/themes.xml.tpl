<resources>
    <style name="Theme.WaterUIApp" parent="Theme.Material3.DayNight.NoActionBar">
{%- for item in theme_items %}
        <item name="{{ item.attr }}">@color/{{ item.color_name }}</item>
{%- endfor %}
    </style>
    <!-- The launch screen: shown by the system from the tap on the icon until
         the activity's first frame, then replaced by Theme.WaterUIApp. -->
    <style name="Theme.WaterUIApp.Launch" parent="Theme.SplashScreen">
        <item name="postSplashScreenTheme">@style/Theme.WaterUIApp</item>
{%- if launch_background %}
        <item name="windowSplashScreenBackground">@color/waterui_launch_background</item>
{%- endif %}
{%- if launch_artwork %}
        <item name="windowSplashScreenAnimatedIcon">@drawable/ic_launch_artwork</item>
{%- endif %}
    </style>
</resources>
