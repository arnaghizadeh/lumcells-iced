//! Configure the window of your application in native platforms.

pub mod icon;

pub use icon::Icon;

pub use crate::core::window::*;
pub use crate::runtime::window::*;

pub use crate::runtime::shell::{ActivationPolicy, set_activation_policy};
