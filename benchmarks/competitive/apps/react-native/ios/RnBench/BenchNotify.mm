// Competitive benchmark — RN contestant's `auto` drive handshake.
//
// The runner posts the `dev.bench.begin` Darwin notification inside its
// measure block; the app answers with `dev.bench.done` when the fling
// program finishes. AX queries cannot carry the signal: a workload can
// stall the app's accessibility server for tens of seconds while it
// materializes, and a timed-out query fails the test instead of driving
// it. Darwin notifications need no AX and are identical on every Apple
// platform. JS polls beginObserved() on a 50 ms timer.
#import <React/RCTBridgeModule.h>
#import <notify.h>

@interface BenchNotify : NSObject <RCTBridgeModule>
@end

@implementation BenchNotify {
  int _token;
  int _pending;
}

RCT_EXPORT_MODULE();

+ (BOOL)requiresMainQueueSetup {
  return NO;
}

- (instancetype)init {
  if ((self = [super init])) {
    _token = -1;
    _pending = 0;
    int token = 0;
    if (notify_register_check("dev.bench.begin", &token) == NOTIFY_STATUS_OK) {
      _token = token;
      int fired = 0;
      // Flush a flag left over from an earlier run on this device.
      notify_check(_token, &fired);
    }
  }
  return self;
}

// The token stays armed for the whole run: each `begin` post re-fires it,
// so every XCTest measure iteration re-runs the program. Posts consumed
// while JS is mid-program queue in `_pending` so none are lost.
RCT_EXPORT_METHOD(beginObserved : (RCTPromiseResolveBlock)resolve
                  reject : (RCTPromiseRejectBlock)reject) {
  if (_token >= 0) {
    int fired = 0;
    notify_check(_token, &fired);
    if (fired != 0) {
      _pending += 1;
      // Tell the runner the post was seen so it stops reposting.
      notify_post("dev.bench.ack");
    }
  }
  if (_pending > 0) {
    _pending -= 1;
    resolve(@YES);
  } else {
    resolve(@NO);
  }
}

RCT_EXPORT_METHOD(postDone) {
  notify_post("dev.bench.done");
}

// `step k n=<param> t=<unix>` → NSTemporaryDirectory()/bench-steps.log plus
// a `dev.bench.step` post — the runner slices its xctrace recording by the
// logged times. Identical format in every contestant.
RCT_EXPORT_METHOD(logStep : (nonnull NSNumber *)step n : (nonnull NSNumber *)n) {
  NSString *line = [NSString
      stringWithFormat:@"step %d n=%d t=%.3f\n", step.intValue, n.intValue,
                       [[NSDate date] timeIntervalSince1970]];
  NSString *path = [NSTemporaryDirectory()
      stringByAppendingString:@"bench-steps.log"];
  NSFileManager *fm = [NSFileManager defaultManager];
  if (![fm fileExistsAtPath:path]) {
    [fm createFileAtPath:path contents:nil attributes:nil];
  }
  NSFileHandle *h = [NSFileHandle fileHandleForWritingAtPath:path];
  [h seekToEndOfFile];
  [h writeData:[line dataUsingEncoding:NSUTF8StringEncoding]];
  [h closeFile];
  notify_post("dev.bench.step");
  // The last ladder step: arm `done` natively NOW — past this point the JS
  // thread (and the bridge hop that would carry a JS postDone) can be
  // saturated for the whole hold. W5 has 8 steps, W6 has 7; settle+hold is
  // 5 s nominally, allow the sweeps +2 s.
  NSString *w = [[NSUserDefaults standardUserDefaults]
      stringForKey:@"bench-workload"];
  int last = [w isEqualToString:@"W5"] ? 7 : ([w isEqualToString:@"W6"] ? 6 : -1);
  if (step.intValue == last) {
    dispatch_after(dispatch_time(DISPATCH_TIME_NOW,
                                 (int64_t)(6.0 * NSEC_PER_SEC)),
                   dispatch_get_global_queue(QOS_CLASS_DEFAULT, 0), ^{
      notify_post("dev.bench.done");
    });
  }
}

// The runner stops reposting once it sees done, so posts queued or latched
// at this point are ack-race backlog, not a new window's signal.
RCT_EXPORT_METHOD(discardBegins) {
  _pending = 0;
  if (_token >= 0) {
    int fired = 0;
    notify_check(_token, &fired);
  }
}

@end
