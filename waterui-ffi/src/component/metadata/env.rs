use waterui::Environment;

use crate::waterui_env;

ffi_metadata!(
    Environment,
    *mut waterui_env,
    waterui_metadata_force_as_env,
    waterui_metadata_env_id
);
