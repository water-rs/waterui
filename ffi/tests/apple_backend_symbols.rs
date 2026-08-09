//! Runtime verification that Apple-backend-required `WaterKit` FFI symbols are
//! linked into the final binary.

#![cfg(target_vendor = "apple")]

use waterui_ffi as _;

unsafe extern "C" {
    fn waterkit_audio_apple_media_session_init();
    fn waterkit_audio_apple_media_session_set_metadata();
    fn waterkit_audio_apple_media_session_set_playback_state();
    fn waterkit_audio_apple_media_session_request_audio_focus();
    fn waterkit_audio_apple_media_session_abandon_audio_focus();
    fn waterkit_audio_apple_media_session_clear();
    fn waterkit_audio_apple_media_session_wait_command();
    fn waterkit_audio_apple_media_session_destroy();
}

#[test]
fn apple_backend_media_session_symbols_are_linked() {
    let symbols = [
        waterkit_audio_apple_media_session_init as unsafe extern "C" fn(),
        waterkit_audio_apple_media_session_set_metadata,
        waterkit_audio_apple_media_session_set_playback_state,
        waterkit_audio_apple_media_session_request_audio_focus,
        waterkit_audio_apple_media_session_abandon_audio_focus,
        waterkit_audio_apple_media_session_clear,
        waterkit_audio_apple_media_session_wait_command,
        waterkit_audio_apple_media_session_destroy,
    ];

    std::hint::black_box(symbols);
}
