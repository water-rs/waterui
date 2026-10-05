// Bridging header shared by every Swift target (apps and UI-test runners):
// exposes the libsystem Darwin-notification API used by the bench drive
// handshake (notify_register_check / notify_check / notify_post /
// notify_cancel). The Darwin Swift module does not export these.
#import <notify.h>
