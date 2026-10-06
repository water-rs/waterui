#import "AppDelegate.h"

#import <React/RCTBundleURLProvider.h>
#import <ReactAppDependencyProvider/RCTAppDependencyProvider.h>
#import <notify.h>

@implementation AppDelegate

- (void)applicationDidFinishLaunching:(NSNotification *)notification
{
  self.moduleName = @"RnBench";
  // Launch argument `-bench-workload w2` lands in NSUserDefaults'
  // NSArgumentDomain. Missing or unrecognized workload traps — a wrong
  // page must fail, never silently measure w1. Scrolling is the runner's
  // OS-level input; no drive argument exists anymore.
  NSString *workload = [[NSUserDefaults standardUserDefaults] stringForKey:@"bench-workload"];
  if (workload == nil ||
      ![@[@"w1", @"w2", @"w3", @"w4", @"w5", @"w6"] containsObject:workload]) {
    fprintf(stderr,
            "missing or unrecognized -bench-workload launch argument "
            "(got %s); expected w1..=w6\n",
            workload ? workload.UTF8String : "nil");
    abort();
  }
  NSString *step = [[NSUserDefaults standardUserDefaults] stringForKey:@"bench-step"];
  NSMutableDictionary *props = [@{@"workload": workload} mutableCopy];
  if (step != nil) {
    props[@"step"] = @(step.integerValue);
  }
  self.initialProps = props;

  self.dependencyProvider = [RCTAppDependencyProvider new];

  // Readiness = the workload's content first appearing (React Native
  // posts RCTContentDidAppearNotification once the surface has mounted
  // its first JS content) — the point every contestant posts
  // `dev.bench.ready.<bundle-id>.<w>` at (SwiftUI `onAppear`, AppKit
  // `viewDidAppear`, WaterUI `on_appear`). The runner waits for it
  // instead of a deep AX query on the 10k-row feed. Registered before
  // the surface starts, so the first appearance cannot be missed.
  NSString *readyName = [NSString stringWithFormat:@"dev.bench.ready.%@.%@",
                         NSBundle.mainBundle.bundleIdentifier ?: @"unknown",
                         workload];
  __block id readyObserver = nil;
  readyObserver = [[NSNotificationCenter defaultCenter]
      addObserverForName:@"RCTContentDidAppearNotification"
                  object:nil
                   queue:[NSOperationQueue mainQueue]
              usingBlock:^(NSNotification *note) {
    if (readyObserver == nil) {
      return;
    }
    [[NSNotificationCenter defaultCenter] removeObserver:readyObserver];
    readyObserver = nil;
    notify_post(readyName.UTF8String);
  }];

  return [super applicationDidFinishLaunching:notification];
}

- (NSURL *)sourceURLForBridge:(RCTBridge *)bridge
{
  return [self bundleURL];
}

- (NSURL *)bundleURL
{
#if DEBUG
  return [[RCTBundleURLProvider sharedSettings] jsBundleURLForBundleRoot:@"index"];
#else
  return [[NSBundle mainBundle] URLForResource:@"main" withExtension:@"jsbundle"];
#endif
}

/// This method controls whether the `concurrentRoot`feature of React18 is turned on or off.
///
/// @see: https://reactjs.org/blog/2022/03/29/react-v18.html
/// @note: This requires to be rendering on Fabric (i.e. on the New Architecture).
/// @return: `true` if the `concurrentRoot` feature is enabled. Otherwise, it returns `false`.
- (BOOL)concurrentRootEnabled
{
#ifdef RN_FABRIC_ENABLED
  return true;
#else
  return false;
#endif
}

@end
