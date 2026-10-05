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
  // The runner waits for `dev.bench.ready.<bundle-id>.<W>` to confirm the
  // argument arrived — a deep AX query on the 10k-row feed stalls for
  // minutes, so notify carries the assertion.
  NSString *bid = NSBundle.mainBundle.bundleIdentifier ?: @"unknown";
  notify_post([NSString stringWithFormat:@"dev.bench.ready.%@.%@",
                        bid, workload]
                  .UTF8String);

  NSString *step = [[NSUserDefaults standardUserDefaults] stringForKey:@"bench-step"];
  NSMutableDictionary *props = [@{@"workload": workload} mutableCopy];
  if (step != nil) {
    props[@"step"] = @(step.integerValue);
  }
  self.initialProps = props;

  // The runner asserts this accessibility identifier after launch; retry
  // until a window's content view exists.
  NSString *marker = [NSString stringWithFormat:@"bench-workload-%@", workload];
  for (double d = 0.0; d <= 2.0; d += 0.5) {
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (int64_t)(d * NSEC_PER_SEC)),
                   dispatch_get_main_queue(), ^{
      for (NSWindow *w in NSApp.windows) {
        w.contentView.accessibilityIdentifier = marker;
      }
    });
  }
  self.dependencyProvider = [RCTAppDependencyProvider new];

  // BENCH_READY on stdout marks the first rendered JS frame for
  // external launch timing.
  [[NSNotificationCenter defaultCenter] addObserverForName:@"RCTContentDidAppearNotification"
                                                  object:nil
                                                   queue:[NSOperationQueue mainQueue]
                                              usingBlock:^(NSNotification *note) {
    fwrite("BENCH_READY\n", 1, 12, stdout);
    fflush(stdout);
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
