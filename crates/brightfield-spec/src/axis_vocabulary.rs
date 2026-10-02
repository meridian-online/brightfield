//! Which of Mosaic's axis attributes brightfield reads.
//!
//! A plot's attributes are an open bag, so a key no resolver asks for is
//! carried and dropped without a word: an author who sets `xTickRotate: 45`
//! sees unrotated ticks and cannot tell a typing mistake from a gap in
//! brightfield. Two lists settle it.
//!
//! - [`SCHEMA_AXIS_ATTRIBUTES`] is Mosaic's: the axis attribute names its
//!   published schema declares on a plot, generated at build time from
//!   `vendor/mosaic-schema/` by `build.rs` through
//!   [`schema::schema_axis_attribute_names`].
//! - [`READ_AXIS_ATTRIBUTES`] is brightfield's: the names the resolvers in
//!   [`crate::layout`] read.
//!
//! [`unread_axis_attributes`] is what the parser asks, for each plot's own keys
//! and once for `plotDefaults:`, and each name it returns becomes a
//! [`crate::ParseWarning::UnreadAxisAttribute`]. A name absent from the schema
//! is not an axis attribute and is not this module's to judge, so a schema that
//! drops a name stops the warning for it.

pub mod schema;

pub use schema::{is_axis_attribute_name, schema_axis_attribute_names};

include!(concat!(env!("OUT_DIR"), "/schema_axis_attributes.rs"));

/// The axis attributes brightfield reads, each by a resolver in
/// [`crate::layout`]: the insets ([`crate::layout::resolve_plot_insets`]), the
/// titles, the fixed domains, the tick counts and formats, the gridlines, the
/// axis ends, the reverse switches and the scale types.
///
/// A card that teaches a resolver a new axis attribute adds its name here in
/// the same edit; `the_read_list_is_what_the_layout_resolvers_read` in
/// `tests/axis_vocabulary_unread.rs` sets each schema name on a plot and fails
/// when a name changes what a resolver returns and is missing here, or is here
/// and changes nothing.
pub const READ_AXIS_ATTRIBUTES: &[&str] = &[
    "grid",
    "xDomain",
    "xGrid",
    "xInset",
    "xInsetLeft",
    "xInsetRight",
    "xLabel",
    "xNice",
    "xReverse",
    "xScale",
    "xTickFormat",
    "xTicks",
    "xZero",
    "yDomain",
    "yGrid",
    "yInset",
    "yInsetBottom",
    "yInsetTop",
    "yLabel",
    "yNice",
    "yReverse",
    "yScale",
    "yTickFormat",
    "yTicks",
    "yZero",
];

/// The keys among `keys` that `schema_names` lists as axis attributes and
/// [`READ_AXIS_ATTRIBUTES`] does not, in the order given.
///
/// The parser passes [`SCHEMA_AXIS_ATTRIBUTES`]; a caller holding the names of
/// some other schema passes those, which is how a test shows the list is the
/// schema's.
pub fn unread_axis_attributes<'a>(
    keys: impl IntoIterator<Item = &'a str>,
    schema_names: &[&str],
) -> Vec<&'a str> {
    keys.into_iter()
        .filter(|key| schema_names.contains(key) && !READ_AXIS_ATTRIBUTES.contains(key))
        .collect()
}
