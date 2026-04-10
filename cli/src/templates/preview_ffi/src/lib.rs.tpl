//! Preview-only wrapper crate for {{ ctx.app_display_name }}.

use {{ ctx.crate_name_ident() }} as app_crate;

// Link the user crate so its `#[preview]` exports are retained in the final dylib.
#[allow(unused_imports)]
use app_crate as _;

