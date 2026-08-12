//! `stubby-controller` — reactive auto-rescue controller (experimental).
//!
//! Watches pods annotated `stubby.io/auto-rescue: "true"`; stubs those stuck
//! in `ImagePullBackOff` and reverts them once the real image is available.
pub mod annotations;
pub mod auth;
pub mod config;
pub mod decision;
pub mod imageref;
pub mod observability;
pub mod reconcile;
pub mod registry;
