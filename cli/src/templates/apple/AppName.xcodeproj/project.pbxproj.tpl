// !$*UTF8*$!
{
	archiveVersion = 1;
	classes = {
	};
	objectVersion = 77;
	objects = {

/* Begin PBXBuildFile section */
		D01867772E6C81BF00802E96 /* lib__LIB_NAME__.a in Frameworks */ = {isa = PBXBuildFile; fileRef = D01867752E6C818300802E96 /* lib__LIB_NAME__.a */; };
		D018677A2E6C82CA00802E96 /* WaterUI in Frameworks */ = {isa = PBXBuildFile; productRef = D01867792E6C82CA00802E96 /* WaterUI */; settings = {ATTRIBUTES = (Required, ); }; };
/* End PBXBuildFile section */

/* Begin PBXFileReference section */
		D018675E2E6C7BBB00802E96 /* __APP_NAME__.app */ = {isa = PBXFileReference; explicitFileType = wrapper.application; includeInIndex = 0; path = __APP_NAME__.app; sourceTree = BUILT_PRODUCTS_DIR; };
		D01867752E6C818300802E96 /* lib__LIB_NAME__.a */ = {isa = PBXFileReference; lastKnownFileType = archive.ar; name = lib__LIB_NAME__.a; path = "$(BUILT_PRODUCTS_DIR)/lib__LIB_NAME__.a"; sourceTree = "<group>"; };
/* End PBXFileReference section */

/* Begin PBXFileSystemSynchronizedRootGroup section */
		D01867602E6C7BBB00802E96 /* __APP_NAME__ */ = {
			isa = PBXFileSystemSynchronizedRootGroup;
			path = __APP_NAME__;
			sourceTree = "<group>";
		};
/* End PBXFileSystemSynchronizedRootGroup section */

/* Begin PBXFrameworksBuildPhase section */
		D018675B2E6C7BBB00802E96 /* Frameworks */ = {
			isa = PBXFrameworksBuildPhase;
			buildActionMask = 2147483647;
			files = (
				D018677A2E6C82CA00802E96 /* WaterUI in Frameworks */,
				D01867772E6C81BF00802E96 /* lib__LIB_NAME__.a in Frameworks */,
			);
			runOnlyForDeploymentPostprocessing = 0;
		};
/* End PBXFrameworksBuildPhase section */

/* Begin PBXGroup section */
		D01867552E6C7BBB00802E96 = {
			isa = PBXGroup;
			children = (
				D01867602E6C7BBB00802E96 /* __APP_NAME__ */,
				D01867742E6C818200802E96 /* Frameworks */,
				D018675F2E6C7BBB00802E96 /* Products */,
			);
			sourceTree = "<group>";
		};
		D018675F2E6C7BBB00802E96 /* Products */ = {
			isa = PBXGroup;
			children = (
				D018675E2E6C7BBB00802E96 /* __APP_NAME__.app */,
			);
			name = Products;
			sourceTree = "<group>";
		};
		D01867742E6C818200802E96 /* Frameworks */ = {
			isa = PBXGroup;
			children = (
				D01867752E6C818300802E96 /* lib__LIB_NAME__.a */,
			);
			name = Frameworks;
			sourceTree = "<group>";
		};
/* End PBXGroup section */

/* Begin PBXNativeTarget section */
		D018675D2E6C7BBB00802E96 /* __APP_NAME__ */ = {
			isa = PBXNativeTarget;
			buildConfigurationList = D018676A2E6C7BBD00802E96 /* Build configuration list for PBXNativeTarget "__APP_NAME__" */;
			buildPhases = (
				D0000001000000000000001 /* Build Rust Library */,
				D018675A2E6C7BBB00802E96 /* Sources */,
				D018675B2E6C7BBB00802E96 /* Frameworks */,
				D018675C2E6C7BBB00802E96 /* Resources */,
			);
			buildRules = (
			);
			dependencies = (
			);
			fileSystemSynchronizedGroups = (
				D01867602E6C7BBB00802E96 /* __APP_NAME__ */,
			);
			name = __APP_NAME__;
			packageProductDependencies = (
				D01867792E6C82CA00802E96 /* WaterUI */,
			);
			productName = __APP_NAME__;
			productReference = D018675E2E6C7BBB00802E96 /* __APP_NAME__.app */;
			productType = "com.apple.product-type.application";
		};
/* End PBXNativeTarget section */

/* Begin PBXProject section */
		D01867562E6C7BBB00802E96 /* Project object */ = {
			isa = PBXProject;
			attributes = {
				BuildIndependentTargetsInParallel = 1;
				LastSwiftUpdateCheck = 1640;
				LastUpgradeCheck = 1640;
				TargetAttributes = {
					D018675D2E6C7BBB00802E96 = {
						CreatedOnToolsVersion = 16.4;
					};
				};
			};
			buildConfigurationList = D01867592E6C7BBB00802E96 /* Build configuration list for PBXProject "__APP_NAME__" */;
			developmentRegion = en;
			hasScannedForEncodings = 0;
			knownRegions = (
				en,
				Base,
			);
			mainGroup = D01867552E6C7BBB00802E96;
			minimizedProjectReferenceProxies = 1;
			packageReferences = (
				D01867782E6C82CA00802E96 /* XCLocalSwiftPackageReference "WaterUI" */,
			);
			preferredProjectObjectVersion = 77;
			productRefGroup = D018675F2E6C7BBB00802E96 /* Products */;
			projectDirPath = "";
			projectRoot = "";
			targets = (
				D018675D2E6C7BBB00802E96 /* __APP_NAME__ */,
			);
		};
/* End PBXProject section */

/* Begin PBXResourcesBuildPhase section */
		D018675C2E6C7BBB00802E96 /* Resources */ = {
			isa = PBXResourcesBuildPhase;
			buildActionMask = 2147483647;
			files = (
			);
			runOnlyForDeploymentPostprocessing = 0;
		};
/* End PBXResourcesBuildPhase section */

/* Begin PBXShellScriptBuildPhase section */
		D0000001000000000000001 /* Build Rust Library */ = {
			isa = PBXShellScriptBuildPhase;
			alwaysOutOfDate = 1;
			buildActionMask = 2147483647;
			files = (
			);
			inputFileListPaths = (
			);
			inputPaths = (
				"$(SRCROOT)/../Cargo.toml",
				"$(SRCROOT)/../src/lib.rs",
			);
			name = "Build Rust Library";
			outputFileListPaths = (
			);
			outputPaths = (
				"$(BUILT_PRODUCTS_DIR)/lib__LIB_NAME__.a",
			);
			runOnlyForDeploymentPostprocessing = 0;
			shellPath = /bin/bash;
			shellScript = "bash \"${PROJECT_DIR}/build-rust.sh\"";
		};
/* End PBXShellScriptBuildPhase section */

/* Begin PBXSourcesBuildPhase section */
		D018675A2E6C7BBB00802E96 /* Sources */ = {
			isa = PBXSourcesBuildPhase;
			buildActionMask = 2147483647;
			files = (
			);
			runOnlyForDeploymentPostprocessing = 0;
		};
/* End PBXSourcesBuildPhase section */

/* Begin XCBuildConfiguration section */
		D01867682E6C7BBD00802E96 /* Debug */ = {
			isa = XCBuildConfiguration;
			buildSettings = {
				ALWAYS_SEARCH_USER_PATHS = NO;
				ASSETCATALOG_COMPILER_GENERATE_SWIFT_ASSET_SYMBOL_EXTENSIONS = YES;
				CLANG_ANALYZER_NONNULL = YES;
				CLANG_ANALYZER_NUMBER_OBJECT_CONVERSION = YES_AGGRESSIVE;
				CLANG_CXX_LANGUAGE_STANDARD = "gnu++20";
				CLANG_ENABLE_MODULES = YES;
				CLANG_ENABLE_OBJC_ARC = YES;
				CLANG_ENABLE_OBJC_WEAK = YES;
				CLANG_WARN_BLOCK_CAPTURE_AUTORELEASING = YES;
				CLANG_WARN_BOOL_CONVERSION = YES;
				CLANG_WARN_COMMA = YES;
				CLANG_WARN_CONSTANT_CONVERSION = YES;
				CLANG_WARN_DEPRECATED_OBJC_IMPLEMENTATIONS = YES;
				CLANG_WARN_DIRECT_OBJC_ISA_USAGE = YES_ERROR;
				CLANG_WARN_DOCUMENTATION_COMMENTS = YES;
				CLANG_WARN_EMPTY_BODY = YES;
				CLANG_WARN_ENUM_CONVERSION = YES;
				CLANG_WARN_INFINITE_RECURSION = YES;
				CLANG_WARN_INT_CONVERSION = YES;
				CLANG_WARN_NON_LITERAL_NULL_CONVERSION = YES;
				CLANG_WARN_OBJC_IMPLICIT_RETAIN_SELF = YES;
				CLANG_WARN_OBJC_LITERAL_CONVERSION = YES;
				CLANG_WARN_OBJC_ROOT_CLASS = YES_ERROR;
				CLANG_WARN_QUOTED_INCLUDE_IN_FRAMEWORK_HEADER = YES;
				CLANG_WARN_RANGE_LOOP_ANALYSIS = YES;
				CLANG_WARN_STRICT_PROTOTYPES = YES;
				CLANG_WARN_SUSPICIOUS_MOVE = YES;
				CLANG_WARN_UNGUARDED_AVAILABILITY = YES_AGGRESSIVE;
				CLANG_WARN_UNREACHABLE_CODE = YES;
				CLANG_WARN__DUPLICATE_METHOD_MATCH = YES;
				COPY_PHASE_STRIP = NO;
				DEBUG_INFORMATION_FORMAT = dwarf;
				DEVELOPMENT_TEAM = "";
				ENABLE_STRICT_OBJC_MSGSEND = YES;
				ENABLE_TESTABILITY = YES;
				ENABLE_USER_SCRIPT_SANDBOXING = YES;
				GCC_C_LANGUAGE_STANDARD = gnu17;
				GCC_DYNAMIC_NO_PIC = NO;
				GCC_NO_COMMON_BLOCKS = YES;
				GCC_OPTIMIZATION_LEVEL = 0;
				GCC_PREPROCESSOR_DEFINITIONS = (
					"DEBUG=1",
					"$(inherited)",
				);
				GCC_WARN_64_TO_32_BIT_CONVERSION = YES;
				GCC_WARN_ABOUT_RETURN_TYPE = YES_ERROR;
				GCC_WARN_UNDECLARED_SELECTOR = YES;
				GCC_WARN_UNINITIALIZED_AUTOS = YES_AGGRESSIVE;
				GCC_WARN_UNUSED_FUNCTION = YES;
				GCC_WARN_UNUSED_VARIABLE = YES;
				LOCALIZATION_PREFERS_STRING_CATALOGS = YES;
				MTL_ENABLE_DEBUG_INFO = INCLUDE_SOURCE;
				MTL_FAST_MATH = YES;
				ONLY_ACTIVE_ARCH = YES;
				SWIFT_ACTIVE_COMPILATION_CONDITIONS = "DEBUG $(inherited)";
				SWIFT_OPTIMIZATION_LEVEL = "-Onone";
			};
			name = Debug;
		};
		D01867692E6C7BBD00802E96 /* Release */ = {
			isa = XCBuildConfiguration;
			buildSettings = {
				ALWAYS_SEARCH_USER_PATHS = NO;
				ASSETCATALOG_COMPILER_GENERATE_SWIFT_ASSET_SYMBOL_EXTENSIONS = YES;
				CLANG_ANALYZER_NONNULL = YES;
				CLANG_ANALYZER_NUMBER_OBJECT_CONVERSION = YES_AGGRESSIVE;
				CLANG_CXX_LANGUAGE_STANDARD = "gnu++20";
				CLANG_ENABLE_MODULES = YES;
				CLANG_ENABLE_OBJC_ARC = YES;
				CLANG_ENABLE_OBJC_WEAK = YES;
				CLANG_WARN_BLOCK_CAPTURE_AUTORELEASING = YES;
				CLANG_WARN_BOOL_CONVERSION = YES;
				CLANG_WARN_COMMA = YES;
				CLANG_WARN_CONSTANT_CONVERSION = YES;
				CLANG_WARN_DEPRECATED_OBJC_IMPLEMENTATIONS = YES;
				CLANG_WARN_DIRECT_OBJC_ISA_USAGE = YES_ERROR;
				CLANG_WARN_DOCUMENTATION_COMMENTS = YES;
				CLANG_WARN_EMPTY_BODY = YES;
				CLANG_WARN_ENUM_CONVERSION = YES;
				CLANG_WARN_INFINITE_RECURSION = YES;
				CLANG_WARN_INT_CONVERSION = YES;
				CLANG_WARN_NON_LITERAL_NULL_CONVERSION = YES;
				CLANG_WARN_OBJC_IMPLICIT_RETAIN_SELF = YES;
				CLANG_WARN_OBJC_LITERAL_CONVERSION = YES;
				CLANG_WARN_OBJC_ROOT_CLASS = YES_ERROR;
				CLANG_WARN_QUOTED_INCLUDE_IN_FRAMEWORK_HEADER = YES;
				CLANG_WARN_RANGE_LOOP_ANALYSIS = YES;
				CLANG_WARN_STRICT_PROTOTYPES = YES;
				CLANG_WARN_SUSPICIOUS_MOVE = YES;
				CLANG_WARN_UNGUARDED_AVAILABILITY = YES_AGGRESSIVE;
				CLANG_WARN_UNREACHABLE_CODE = YES;
				CLANG_WARN__DUPLICATE_METHOD_MATCH = YES;
				COPY_PHASE_STRIP = NO;
				DEBUG_INFORMATION_FORMAT = "dwarf-with-dsym";
				DEVELOPMENT_TEAM = "";
				ENABLE_NS_ASSERTIONS = NO;
				ENABLE_STRICT_OBJC_MSGSEND = YES;
				ENABLE_USER_SCRIPT_SANDBOXING = YES;
				GCC_C_LANGUAGE_STANDARD = gnu17;
				GCC_NO_COMMON_BLOCKS = YES;
				GCC_WARN_64_TO_32_BIT_CONVERSION = YES;
				GCC_WARN_ABOUT_RETURN_TYPE = YES_ERROR;
				GCC_WARN_UNDECLARED_SELECTOR = YES;
				GCC_WARN_UNINITIALIZED_AUTOS = YES_AGGRESSIVE;
				GCC_WARN_UNUSED_FUNCTION = YES;
				GCC_WARN_UNUSED_VARIABLE = YES;
				LOCALIZATION_PREFERS_STRING_CATALOGS = YES;
				MTL_ENABLE_DEBUG_INFO = NO;
				MTL_FAST_MATH = YES;
				SWIFT_COMPILATION_MODE = wholemodule;
			};
			name = Release;
		};
		D018676B2E6C7BBD00802E96 /* Debug */ = {
			isa = XCBuildConfiguration;
			buildSettings = {
				ASSETCATALOG_COMPILER_APPICON_NAME = AppIcon;
				ASSETCATALOG_COMPILER_GLOBAL_ACCENT_COLOR_NAME = AccentColor;
				AUTOMATION_APPLE_EVENTS = NO;
				CODE_SIGN_ENTITLEMENTS = "__APP_NAME__/__APP_NAME__.entitlements";
				CODE_SIGN_STYLE = Automatic;
				CURRENT_PROJECT_VERSION = 1;
				DEVELOPMENT_TEAM = "";
				ENABLE_APP_SANDBOX = NO;
				ENABLE_HARDENED_RUNTIME = NO;
				ENABLE_PREVIEWS = YES;
				ENABLE_RESOURCE_ACCESS_AUDIO_INPUT = NO;
				ENABLE_RESOURCE_ACCESS_CALENDARS = NO;
				ENABLE_RESOURCE_ACCESS_CAMERA = NO;
				ENABLE_RESOURCE_ACCESS_CONTACTS = NO;
				ENABLE_RESOURCE_ACCESS_LOCATION = NO;
				ENABLE_RESOURCE_ACCESS_PHOTO_LIBRARY = NO;
				GENERATE_INFOPLIST_FILE = YES;
				"INFOPLIST_KEY_UIApplicationSceneManifest_Generation[sdk=iphoneos*]" = YES;
				"INFOPLIST_KEY_UIApplicationSceneManifest_Generation[sdk=iphonesimulator*]" = YES;
				"INFOPLIST_KEY_UIApplicationSupportsIndirectInputEvents[sdk=iphoneos*]" = YES;
				"INFOPLIST_KEY_UIApplicationSupportsIndirectInputEvents[sdk=iphonesimulator*]" = YES;
				"INFOPLIST_KEY_UILaunchScreen_Generation[sdk=iphoneos*]" = YES;
				"INFOPLIST_KEY_UILaunchScreen_Generation[sdk=iphonesimulator*]" = YES;
				"INFOPLIST_KEY_UIStatusBarStyle[sdk=iphoneos*]" = UIStatusBarStyleDefault;
				"INFOPLIST_KEY_UIStatusBarStyle[sdk=iphonesimulator*]" = UIStatusBarStyleDefault;
				INFOPLIST_KEY_UISupportedInterfaceOrientations_iPad = "UIInterfaceOrientationPortrait UIInterfaceOrientationPortraitUpsideDown UIInterfaceOrientationLandscapeLeft UIInterfaceOrientationLandscapeRight";
				INFOPLIST_KEY_UISupportedInterfaceOrientations_iPhone = "UIInterfaceOrientationPortrait UIInterfaceOrientationLandscapeLeft UIInterfaceOrientationLandscapeRight";
				IPHONEOS_DEPLOYMENT_TARGET = 18.5;
				LD_RUNPATH_SEARCH_PATHS = "@executable_path/Frameworks";
				"LD_RUNPATH_SEARCH_PATHS[sdk=macosx*]" = "@executable_path/../Frameworks";
				LIBRARY_SEARCH_PATHS = "$(BUILT_PRODUCTS_DIR)";
				MACOSX_DEPLOYMENT_TARGET = 26.0;
				MARKETING_VERSION = 1.0;
				OTHER_LDFLAGS = "-l__LIB_NAME__";
				PRODUCT_BUNDLE_IDENTIFIER = __BUNDLE_IDENTIFIER__;
				PRODUCT_NAME = "$(TARGET_NAME)";
				REGISTER_APP_GROUPS = YES;
				RUNTIME_EXCEPTION_ALLOW_DYLD_ENVIRONMENT_VARIABLES = NO;
				RUNTIME_EXCEPTION_ALLOW_JIT = NO;
				RUNTIME_EXCEPTION_ALLOW_UNSIGNED_EXECUTABLE_MEMORY = NO;
				RUNTIME_EXCEPTION_DEBUGGING_TOOL = NO;
				RUNTIME_EXCEPTION_DISABLE_EXECUTABLE_PAGE_PROTECTION = NO;
				RUNTIME_EXCEPTION_DISABLE_LIBRARY_VALIDATION = NO;
				SDKROOT = auto;
				SUPPORTED_PLATFORMS = "iphoneos iphonesimulator macosx xros xrsimulator";
				SWIFT_EMIT_LOC_STRINGS = YES;
				SWIFT_VERSION = 5.0;
				TARGETED_DEVICE_FAMILY = "1,2,7";
				XROS_DEPLOYMENT_TARGET = 2.5;
			};
			name = Debug;
		};
		D018676C2E6C7BBD00802E96 /* Release */ = {
			isa = XCBuildConfiguration;
			buildSettings = {
				ASSETCATALOG_COMPILER_APPICON_NAME = AppIcon;
				ASSETCATALOG_COMPILER_GLOBAL_ACCENT_COLOR_NAME = AccentColor;
				AUTOMATION_APPLE_EVENTS = NO;
				CODE_SIGN_ENTITLEMENTS = "__APP_NAME__/__APP_NAME__.entitlements";
				CODE_SIGN_STYLE = Automatic;
				CURRENT_PROJECT_VERSION = 1;
				DEVELOPMENT_TEAM = "";
				ENABLE_APP_SANDBOX = NO;
				ENABLE_HARDENED_RUNTIME = NO;
				ENABLE_PREVIEWS = YES;
				ENABLE_RESOURCE_ACCESS_AUDIO_INPUT = NO;
				ENABLE_RESOURCE_ACCESS_CALENDARS = NO;
				ENABLE_RESOURCE_ACCESS_CAMERA = NO;
				ENABLE_RESOURCE_ACCESS_CONTACTS = NO;
				ENABLE_RESOURCE_ACCESS_LOCATION = NO;
				ENABLE_RESOURCE_ACCESS_PHOTO_LIBRARY = NO;
				GENERATE_INFOPLIST_FILE = YES;
				"INFOPLIST_KEY_UIApplicationSceneManifest_Generation[sdk=iphoneos*]" = YES;
				"INFOPLIST_KEY_UIApplicationSceneManifest_Generation[sdk=iphonesimulator*]" = YES;
				"INFOPLIST_KEY_UIApplicationSupportsIndirectInputEvents[sdk=iphoneos*]" = YES;
				"INFOPLIST_KEY_UIApplicationSupportsIndirectInputEvents[sdk=iphonesimulator*]" = YES;
				"INFOPLIST_KEY_UILaunchScreen_Generation[sdk=iphoneos*]" = YES;
				"INFOPLIST_KEY_UILaunchScreen_Generation[sdk=iphonesimulator*]" = YES;
				"INFOPLIST_KEY_UIStatusBarStyle[sdk=iphoneos*]" = UIStatusBarStyleDefault;
				"INFOPLIST_KEY_UIStatusBarStyle[sdk=iphonesimulator*]" = UIStatusBarStyleDefault;
				INFOPLIST_KEY_UISupportedInterfaceOrientations_iPad = "UIInterfaceOrientationPortrait UIInterfaceOrientationPortraitUpsideDown UIInterfaceOrientationLandscapeLeft UIInterfaceOrientationLandscapeRight";
				INFOPLIST_KEY_UISupportedInterfaceOrientations_iPhone = "UIInterfaceOrientationPortrait UIInterfaceOrientationLandscapeLeft UIInterfaceOrientationLandscapeRight";
				IPHONEOS_DEPLOYMENT_TARGET = 18.5;
				LD_RUNPATH_SEARCH_PATHS = "@executable_path/Frameworks";
				"LD_RUNPATH_SEARCH_PATHS[sdk=macosx*]" = "@executable_path/../Frameworks";
				LIBRARY_SEARCH_PATHS = "$(BUILT_PRODUCTS_DIR)";
				MACOSX_DEPLOYMENT_TARGET = 26.0;
				MARKETING_VERSION = 1.0;
				OTHER_LDFLAGS = "-l__LIB_NAME__";
				PRODUCT_BUNDLE_IDENTIFIER = __BUNDLE_IDENTIFIER__;
				PRODUCT_NAME = "$(TARGET_NAME)";
				REGISTER_APP_GROUPS = YES;
				RUNTIME_EXCEPTION_ALLOW_DYLD_ENVIRONMENT_VARIABLES = NO;
				RUNTIME_EXCEPTION_ALLOW_JIT = NO;
				RUNTIME_EXCEPTION_ALLOW_UNSIGNED_EXECUTABLE_MEMORY = NO;
				RUNTIME_EXCEPTION_DEBUGGING_TOOL = NO;
				RUNTIME_EXCEPTION_DISABLE_EXECUTABLE_PAGE_PROTECTION = NO;
				RUNTIME_EXCEPTION_DISABLE_LIBRARY_VALIDATION = NO;
				SDKROOT = auto;
				SUPPORTED_PLATFORMS = "iphoneos iphonesimulator macosx xros xrsimulator";
				SWIFT_EMIT_LOC_STRINGS = YES;
				SWIFT_VERSION = 5.0;
				TARGETED_DEVICE_FAMILY = "1,2,7";
				XROS_DEPLOYMENT_TARGET = 2.5;
			};
			name = Release;
		};
/* End XCBuildConfiguration section */

/* Begin XCConfigurationList section */
		D01867592E6C7BBB00802E96 /* Build configuration list for PBXProject "__APP_NAME__" */ = {
			isa = XCConfigurationList;
			buildConfigurations = (
				D01867682E6C7BBD00802E96 /* Debug */,
				D01867692E6C7BBD00802E96 /* Release */,
			);
			defaultConfigurationIsVisible = 0;
			defaultConfigurationName = Release;
		};
		D018676A2E6C7BBD00802E96 /* Build configuration list for PBXNativeTarget "__APP_NAME__" */ = {
			isa = XCConfigurationList;
			buildConfigurations = (
				D018676B2E6C7BBD00802E96 /* Debug */,
				D018676C2E6C7BBD00802E96 /* Release */,
			);
			defaultConfigurationIsVisible = 0;
			defaultConfigurationName = Release;
		};
/* End XCConfigurationList section */

/* Begin XCLocalSwiftPackageReference section */
		D01867782E6C82CA00802E96 /* XCLocalSwiftPackageReference "WaterUI" */ = {
			isa = XCLocalSwiftPackageReference;
			relativePath = WaterUI;
		};
/* End XCLocalSwiftPackageReference section */

/* Begin XCSwiftPackageProductDependency section */
		D01867792E6C82CA00802E96 /* WaterUI */ = {
			isa = XCSwiftPackageProductDependency;
			productName = WaterUI;
		};
/* End XCSwiftPackageProductDependency section */
	};
	rootObject = D01867562E6C7BBB00802E96 /* Project object */;
}
