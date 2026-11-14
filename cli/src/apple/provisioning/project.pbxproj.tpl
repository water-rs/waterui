// !$*UTF8*$!
{
	archiveVersion = 1;
	classes = {
	};
	objectVersion = 56;
	objects = {

/* Begin PBXBuildFile section */
		AA0000000000000000000001 /* main.swift in Sources */ = {isa = PBXBuildFile; fileRef = AA0000000000000000000003 /* main.swift */; };
/* End PBXBuildFile section */

/* Begin PBXFileReference section */
		AA0000000000000000000002 /* Provision.app */ = {isa = PBXFileReference; explicitFileType = wrapper.application; includeInIndex = 0; path = Provision.app; sourceTree = BUILT_PRODUCTS_DIR; };
		AA0000000000000000000003 /* main.swift */ = {isa = PBXFileReference; lastKnownFileType = sourcecode.swift; path = main.swift; sourceTree = "<group>"; };
		AA0000000000000000000004 /* Provision.entitlements */ = {isa = PBXFileReference; lastKnownFileType = text.plist.entitlements; path = Provision.entitlements; sourceTree = "<group>"; };
/* End PBXFileReference section */

/* Begin PBXFrameworksBuildPhase section */
		AA0000000000000000000005 /* Frameworks */ = {
			isa = PBXFrameworksBuildPhase;
			buildActionMask = 2147483647;
			files = (
			);
			runOnlyForDeploymentPostprocessing = 0;
		};
/* End PBXFrameworksBuildPhase section */

/* Begin PBXGroup section */
		AA0000000000000000000006 = {
			isa = PBXGroup;
			children = (
				AA0000000000000000000012 /* Provision */,
				AA0000000000000000000007 /* Products */,
			);
			sourceTree = "<group>";
		};
		AA0000000000000000000007 /* Products */ = {
			isa = PBXGroup;
			children = (
				AA0000000000000000000002 /* Provision.app */,
			);
			name = Products;
			sourceTree = "<group>";
		};
		AA0000000000000000000012 /* Provision */ = {
			isa = PBXGroup;
			children = (
				AA0000000000000000000003 /* main.swift */,
				AA0000000000000000000004 /* Provision.entitlements */,
			);
			path = Provision;
			sourceTree = "<group>";
		};
/* End PBXGroup section */

/* Begin PBXNativeTarget section */
		AA0000000000000000000008 /* Provision */ = {
			isa = PBXNativeTarget;
			buildConfigurationList = AA0000000000000000000009 /* Build configuration list for PBXNativeTarget "Provision" */;
			buildPhases = (
				AA000000000000000000000A /* Sources */,
				AA0000000000000000000005 /* Frameworks */,
				AA000000000000000000000B /* Resources */,
			);
			buildRules = (
			);
			dependencies = (
			);
			name = Provision;
			productName = Provision;
			productReference = AA0000000000000000000002 /* Provision.app */;
			productType = "com.apple.product-type.application";
		};
/* End PBXNativeTarget section */

/* Begin PBXProject section */
		AA000000000000000000000C /* Project object */ = {
			isa = PBXProject;
			attributes = {
				BuildIndependentTargetsInParallel = 1;
				LastUpgradeCheck = 1600;
				TargetAttributes = {
					AA0000000000000000000008 = {
						CreatedOnToolsVersion = 16.0;
					};
				};
			};
			buildConfigurationList = AA000000000000000000000D /* Build configuration list for PBXProject "Provision" */;
			compatibilityVersion = "Xcode 14.0";
			developmentRegion = en;
			hasScannedForEncodings = 0;
			knownRegions = (
				en,
				Base,
			);
			mainGroup = AA0000000000000000000006;
			productRefGroup = AA0000000000000000000007 /* Products */;
			projectDirPath = "";
			projectRoot = "";
			targets = (
				AA0000000000000000000008 /* Provision */,
			);
		};
/* End PBXProject section */

/* Begin PBXResourcesBuildPhase section */
		AA000000000000000000000B /* Resources */ = {
			isa = PBXResourcesBuildPhase;
			buildActionMask = 2147483647;
			files = (
			);
			runOnlyForDeploymentPostprocessing = 0;
		};
/* End PBXResourcesBuildPhase section */

/* Begin PBXSourcesBuildPhase section */
		AA000000000000000000000A /* Sources */ = {
			isa = PBXSourcesBuildPhase;
			buildActionMask = 2147483647;
			files = (
				AA0000000000000000000001 /* main.swift in Sources */,
			);
			runOnlyForDeploymentPostprocessing = 0;
		};
/* End PBXSourcesBuildPhase section */

/* Begin XCBuildConfiguration section */
		AA000000000000000000000E /* Debug */ = {
			isa = XCBuildConfiguration;
			buildSettings = {
				{{ deployment_setting }} = {{ deployment_target }};
				SDKROOT = {{ sdkroot }};
				SWIFT_OPTIMIZATION_LEVEL = "-Onone";
			};
			name = Debug;
		};
		AA000000000000000000000F /* Release */ = {
			isa = XCBuildConfiguration;
			buildSettings = {
				{{ deployment_setting }} = {{ deployment_target }};
				SDKROOT = {{ sdkroot }};
				SWIFT_COMPILATION_MODE = wholemodule;
				VALIDATE_PRODUCT = YES;
			};
			name = Release;
		};
		AA0000000000000000000010 /* Debug */ = {
			isa = XCBuildConfiguration;
			buildSettings = {
				CODE_SIGN_ENTITLEMENTS = Provision/Provision.entitlements;
				CODE_SIGN_STYLE = Automatic;
				CURRENT_PROJECT_VERSION = 1;
				DEVELOPMENT_TEAM = {{ team }};
				ENABLE_USER_SCRIPT_SANDBOXING = NO;
				GENERATE_INFOPLIST_FILE = YES;
				{{ deployment_setting }} = {{ deployment_target }};
				MARKETING_VERSION = 1.0;
				PRODUCT_BUNDLE_IDENTIFIER = {{ bundle_id }};
				PRODUCT_NAME = "$(TARGET_NAME)";
				SDKROOT = {{ sdkroot }};
				SWIFT_EMIT_LOC_STRINGS = YES;
				SWIFT_VERSION = 5.0;
				TARGETED_DEVICE_FAMILY = "{{ device_family }}";
			};
			name = Debug;
		};
		AA0000000000000000000011 /* Release */ = {
			isa = XCBuildConfiguration;
			buildSettings = {
				CODE_SIGN_ENTITLEMENTS = Provision/Provision.entitlements;
				CODE_SIGN_STYLE = Automatic;
				CURRENT_PROJECT_VERSION = 1;
				DEVELOPMENT_TEAM = {{ team }};
				ENABLE_USER_SCRIPT_SANDBOXING = NO;
				GENERATE_INFOPLIST_FILE = YES;
				{{ deployment_setting }} = {{ deployment_target }};
				MARKETING_VERSION = 1.0;
				PRODUCT_BUNDLE_IDENTIFIER = {{ bundle_id }};
				PRODUCT_NAME = "$(TARGET_NAME)";
				SDKROOT = {{ sdkroot }};
				SWIFT_EMIT_LOC_STRINGS = YES;
				SWIFT_VERSION = 5.0;
				TARGETED_DEVICE_FAMILY = "{{ device_family }}";
			};
			name = Release;
		};
/* End XCBuildConfiguration section */

/* Begin XCConfigurationList section */
		AA0000000000000000000009 /* Build configuration list for PBXNativeTarget "Provision" */ = {
			isa = XCConfigurationList;
			buildConfigurations = (
				AA0000000000000000000010 /* Debug */,
				AA0000000000000000000011 /* Release */,
			);
			defaultConfigurationIsVisible = 0;
			defaultConfigurationName = Release;
		};
		AA000000000000000000000D /* Build configuration list for PBXProject "Provision" */ = {
			isa = XCConfigurationList;
			buildConfigurations = (
				AA000000000000000000000E /* Debug */,
				AA000000000000000000000F /* Release */,
			);
			defaultConfigurationIsVisible = 0;
			defaultConfigurationName = Release;
		};
/* End XCConfigurationList section */
	};
	rootObject = AA000000000000000000000C /* Project object */;
}
