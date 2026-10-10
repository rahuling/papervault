//! PaperVault core library.
//!
//! PDF tools that run entirely on your machine. No function here opens a network connection.
//!
//! - [`tools::run`] runs any tool by name (the CLI and web UI both use it).
//! - [`pdf`], [`images`] and [`stamp`] hold the individual operations.

pub mod external;
pub mod images;
pub mod pdf;
pub mod stamp;
pub mod tools;
