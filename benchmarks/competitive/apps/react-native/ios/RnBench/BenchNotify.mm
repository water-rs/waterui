// Competitive benchmark — RN contestant's done-post bridge.
//
// One launch renders one ladder step on every leg; the app's own
// workload logic ends each capacity hold by calling postDone(), which
// carries the `dev.bench.done` Darwin post the runner waits for. No
// begin handshake and no native-side timers live here anymore.
#import <React/RCTBridgeModule.h>
#import <notify.h>

@interface BenchNotify : NSObject <RCTBridgeModule>
@end

@implementation BenchNotify

RCT_EXPORT_MODULE();

+ (BOOL)requiresMainQueueSetup {
  return NO;
}

RCT_EXPORT_METHOD(postDone) {
  notify_post("dev.bench.done");
}

@end
