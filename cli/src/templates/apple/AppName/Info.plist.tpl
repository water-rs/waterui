<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<!-- Xcode merges the generated keys (GENERATE_INFOPLIST_FILE) into this
	     file. UILaunchScreen has no INFOPLIST_KEY_ build setting for its
	     sub-keys, so the launch screen is declared here: an empty dictionary
	     is the system-generated launch screen, the named color and image are
	     the sets the CLI stages into WaterUIAssets.xcassets. -->
	<key>UILaunchScreen</key>
	<dict>
{%- if ctx.launch.has_background %}
		<key>UIColorName</key>
		<string>LaunchBackground</string>
{%- endif %}
{%- if ctx.launch.has_image %}
		<key>UIImageName</key>
		<string>LaunchImage</string>
		<key>UIImageRespectsSafeAreaInsets</key>
		<true/>
{%- endif %}
	</dict>
</dict>
</plist>
