//! HTTP protocol support for Asupersync.
//!
//! This module provides HTTP/1.1 and HTTP/2 protocol implementations
//! with cancel-safe body handling and connection pooling.
//!
//! # Body Types
//!
//! The [`body`] module provides the [`Body`] trait and common
//! implementations for streaming HTTP message bodies.
//!
//! # HTTP/2
//!
//! The [`h2`] module provides HTTP/2 protocol support including frame
//! parsing, HPACK compression, and flow control.
//!
//! # Connection Pooling
//!
//! The [`pool`] module provides connection pool management for HTTP clients,
//! enabling connection reuse for improved performance.

pub mod body;
pub mod compress;
pub mod h1;
pub mod h2;

// Conformance tests for H1 vs H2 header decoder equivalence
#[cfg(test)]
mod h1_h2_header_conformance_test;

pub mod pool;

/// High-level pooled [`Client`] facade.
///
/// Home of the capability-gated `Client::default_for_runtime` accessor — the
/// no-ambient-global entry point to the runtime's default client.
pub mod client;

pub use body::{Body, Empty, Frame, Full, HeaderMap, HeaderName, HeaderValue, SizeHint};
pub use h1::{
    ClientError, ClientRequestBuilder, HttpClient, HttpClientBuilder, HttpClientConfig, Method,
    MultipartError, MultipartForm, ParsedUrl, RedirectPolicy, Request, RequestBuilder, Response,
    ResponseBuilder, RetryPolicy, Scheme, StatusCode, Version,
};

/// Ergonomic, capability-gated handle for the high-level pooled HTTP client.
///
/// See [`client`] for the no-ambient-global philosophy and
/// [`Client::default_for_runtime`](client::Client::default_for_runtime).
pub use client::Client;

pub use pool::{Pool, PoolConfig, PoolKey, PoolStats, PooledConnectionMeta, PooledConnectionState};
