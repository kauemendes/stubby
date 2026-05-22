//! `stubby-webhook` — the Kubernetes Mutating Admission Webhook crate.
//!
//! The crate is split into small focused modules:
//!
//! - [`annotation`] — parses `stubby.io/*` annotations into a [`annotation::Decision`].
//! - [`patch`]      — builds the RFC 6902 JSON Patch overlaid on each container.
//! - [`admission`]  — `admission.k8s.io/v1` AdmissionReview types and the top-level
//!                    [`admission::handle`] decision function.
//! - [`server`]     — axum [`server::router`] wiring `/healthz`, `/readyz`,
//!                    `/metrics`, and `/mutate`.
//! - [`config`]     — startup configuration plumbed through env vars.
//! - [`observability`] — Prometheus recorder + the `stubby_admissions_total` counter.
//! - [`error`]      — error type used internally.
//!
//! The `main` binary glues them together with `tokio` + `axum-server`'s TLS
//! listener, a cert-reload ticker, and SIGTERM/SIGINT graceful shutdown.
pub mod admission;
pub mod annotation;
pub mod config;
pub mod error;
pub mod observability;
pub mod patch;
pub mod server;
