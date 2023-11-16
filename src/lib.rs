#![doc = include_str!("../README.md")]
#![deny(unreachable_pub)]

#[macro_use]
pub mod circuit; // ok
pub mod cli; // ok 
pub mod config; // ok
mod cont;
pub mod coprocessor; // ok
pub mod error; // ok
pub mod eval; // ok
pub mod expr;
pub mod field; // ok
pub mod hash;
pub mod hash_witness;
pub mod lem;
mod num;
pub mod package;
pub mod parser;
pub mod proof;
pub mod ptr;
pub mod public_parameters;
pub mod state;
pub mod store;
pub mod symbol;
pub mod syntax;
mod syntax_macros;
pub mod tag;
pub mod uint;
pub mod writer;
pub mod z_data;
pub use num::Num;
pub use symbol::Symbol;
pub use uint::UInt;

pub use z_data::{z_cont, z_expr, z_ptr, z_store};
