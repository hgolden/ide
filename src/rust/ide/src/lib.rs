//! Main library crate for IDE. It includes implementation of
//! controllers, view logic and code that wraps them all together.
//!
#![feature(arbitrary_self_types)]
#![feature(async_closure)]
#![feature(associated_type_bounds)]
#![feature(bool_to_option)]
#![feature(cell_update)]
#![feature(drain_filter)]
#![feature(exact_size_is_empty)]
#![feature(iter_order_by)]
#![feature(option_result_contains)]
#![feature(trait_alias)]
#![feature(matches_macro)]
#![feature(range_is_empty)]
#![feature(result_cloned)]
#![feature(slice_patterns)]
#![feature(result_map_or_else)]
#![recursion_limit="256"]
#![warn(missing_docs)]
#![warn(trivial_casts)]
#![warn(trivial_numeric_casts)]
#![warn(unused_import_braces)]
#![warn(unused_qualifications)]
#![warn(unsafe_code)]
#![warn(missing_copy_implementations)]
#![warn(missing_debug_implementations)]

pub mod config;
pub mod constants;
pub mod controller;
pub mod double_representation;
pub mod executor;
pub mod ide;
pub mod model;
pub mod notification;
pub mod test;
pub mod transport;

pub use crate::ide::*;

use ensogl::system::web;
// This import is required to have all EnsoGL examples entry points visible in IDE.
#[allow(unused_imports)]
use ensogl_examples::*;
use wasm_bindgen::prelude::*;

#[cfg(test)]
mod tests;

/// Common types that should be visible across the whole IDE crate.
pub mod prelude {
    pub use ensogl::prelude::*;
    pub use ensogl::prelude::DefaultWarningLogger as Logger;
    pub use enso_prelude::*;
    pub use ast::prelude::*;
    pub use wasm_bindgen::prelude::*;

    pub use crate::constants;
    pub use crate::controller;
    pub use crate::double_representation;
    pub use crate::executor;
    pub use crate::model;

    pub use enso_protocol::prelude::BoxFuture;
    pub use enso_protocol::prelude::StaticBoxFuture;
    pub use enso_protocol::prelude::StaticBoxStream;

    pub use futures::Future;
    pub use futures::FutureExt;
    pub use futures::Stream;
    pub use futures::StreamExt;
    pub use futures::task::LocalSpawnExt;

    pub use std::ops::Range;

    pub use utils::fail::FallibleResult;
    pub use utils::vec::VecExt;

    pub use uuid::Uuid;

    #[cfg(test)] pub use wasm_bindgen_test::wasm_bindgen_test;
    #[cfg(test)] pub use wasm_bindgen_test::wasm_bindgen_test_configure;
}

/// IDE startup function.
#[wasm_bindgen]
#[allow(dead_code)]
pub fn entry_point_ide() {
    web::forward_panic_hook_to_error();
    web::set_stdout();

    // FIXME: This code is temporary. It's used to remove the loader UI.
    ensogl_text_msdf_sys::run_once_initialized(|| {

        // Logging of build information.
        #[cfg(debug_assertions)]
            analytics::remote_log_value("debug_mode", "debug_mode_is_active", analytics::AnonymousData(true));
        #[cfg(not(debug_assertions))]
            analytics::remote_log_value("debug_mode", "debug_mode_is_active", analytics::AnonymousData(false));

        let config = crate::config::Startup::from_web_arguments().expect("Failed to read configuration.");
        crate::ide::Initializer::new(config).start_and_forget();
    });
}
