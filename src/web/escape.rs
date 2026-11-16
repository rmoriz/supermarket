//! Rails-compatible HTML escaping for askama.
//!
//! Rails' `ERB::Util.html_escape` maps `& " ' < >` to named/`&#39;` entities;
//! askama's built-in escaper uses numeric entities for all five. Since the
//! differential harness compares entity spelling, this escaper mirrors Rails.

use std::fmt::{self, Write};

#[derive(Debug, Clone, Copy, Default)]
pub struct Rails;

impl askama::filters::Escaper for Rails {
    fn write_escaped_str<W: Write>(&self, mut dest: W, string: &str) -> fmt::Result {
        for byte in string.bytes() {
            match byte {
                b'&' => dest.write_str("&amp;")?,
                b'"' => dest.write_str("&quot;")?,
                b'\'' => dest.write_str("&#39;")?,
                b'<' => dest.write_str("&lt;")?,
                b'>' => dest.write_str("&gt;")?,
                _ => dest.write_char(byte as char)?,
            }
        }
        Ok(())
    }
}
