//! I18n (internationalization) support for Angular templates.
//!
//! This module provides the i18n infrastructure for Angular template compilation,
//! ported 1:1 from Angular's TypeScript implementation.
//!
//! ## Architecture
//!
//! The i18n system consists of:
//!
//! - **AST**: I18n message representation ([`ast`])
//! - **Parser**: Converts HTML nodes to i18n messages ([`parser`])
//! - **Placeholder Registry**: Generates unique placeholder names ([`placeholder`])
//! - **Digest**: Message ID generation for translation lookups ([`digest`])
//! - **Serializer**: Serializes i18n messages for $localize ([`serializer`])
//! - **Serializers**: Translation file format serializers ([`serializers`])
//!   - XLIFF 1.2: Industry-standard translation format
//!   - XLIFF 2.0: Updated XLIFF specification
//!   - XMB: XML Message Bundle (Google Closure format)
//!   - XTB: XML Translation Bundle (companion to XMB)
//! - **Extractor/Merger**: Extract messages from HTML and merge translations ([`extractor_merger`])
//! - **Message Bundle**: Container for extracted messages ([`message_bundle`])
//! - **Translation Bundle**: Load and lookup translations ([`translation_bundle`])
//! - **I18n HTML Parser**: HTML parser with translation injection ([`i18n_html_parser`])
//!
//! Ported from Angular's `compiler/src/i18n/`.

pub mod ast;
pub mod digest;
pub mod extractor_merger;
pub mod i18n_html_parser;
pub mod message_bundle;
pub mod parser;
pub mod placeholder;
pub mod serializer;
pub mod serializers;
pub mod translation_bundle;

// Re-export key types
pub use ast::{
    BlockPlaceholder, Container, Icu, IcuPlaceholder, Message, MessagePlaceholder, MessageSpan,
    Node, Placeholder, TagPlaceholder, Text, Visitor,
};
pub use digest::{
    compute_decimal_digest, compute_digest, compute_msg_id, decimal_digest, fingerprint, sha1,
};
pub use extractor_merger::{
    ExtractionResult, MergeResult, TranslatedNode, extract_messages, merge_translations,
};
pub use i18n_html_parser::{I18nHtmlParser, I18nHtmlParserOptions, MissingTranslationStrategy};
pub use message_bundle::MessageBundle;
pub use parser::{I18nMessageFactory, I18nVisitorContext, create_i18n_message_factory};
pub use placeholder::PlaceholderRegistry;
pub use serializers::{
    LoadResult, PlaceholderMapper, Serializer, SerializerError, SimplePlaceholderMapper,
    Xliff1Serializer, Xliff2Serializer, XmbSerializer, XtbSerializer,
};
pub use translation_bundle::TranslationBundle;
