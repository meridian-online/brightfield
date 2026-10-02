//! Reading Mosaic's axis attribute names out of its published JSON Schema.
//!
//! Compiled twice: into the crate as `axis_vocabulary::schema`, and into
//! `build.rs` by path, which runs it over the vendored schema to generate
//! `SCHEMA_AXIS_ATTRIBUTES`. One function serves both so the list the parser
//! warns from and the list a test derives from an edited schema cannot be
//! derived two different ways. It uses `serde_json` and `std` alone, because
//! those are what the build script is compiled with.

/// Whether a plot attribute name is an axis attribute: the bare `grid`, or `x`
/// or `y` followed by a capital letter, or a facet axis's name, which is that
/// behind an `f` (`fxLabel`, `fyTickFormat`). `xyDomain` has a lower-case second
/// letter and `facetGrid` has no `x` or `y` after its `f`, so neither counts.
#[must_use]
pub fn is_axis_attribute_name(name: &str) -> bool {
    if name == "grid" {
        return true;
    }
    let position = name.strip_prefix('f').unwrap_or(name);
    let mut chars = position.chars();
    matches!(chars.next(), Some('x' | 'y')) && chars.next().is_some_and(|c| c.is_ascii_uppercase())
}

/// The axis attribute names a Mosaic schema declares on a plot, sorted: the
/// property names of `definitions.PlotAttributes.properties` that
/// [`is_axis_attribute_name`] accepts. Sorted here rather than left in map
/// order, because whether `serde_json` keeps a document's key order depends on
/// a feature the build script and the crate may not resolve alike.
///
/// # Errors
/// Names the step that failed when the document has no
/// `definitions.PlotAttributes.properties` object.
pub fn schema_axis_attribute_names(schema: &serde_json::Value) -> Result<Vec<String>, String> {
    let properties = schema
        .get("definitions")
        .ok_or("the schema has no `definitions`")?
        .get("PlotAttributes")
        .ok_or("the schema has no `definitions.PlotAttributes`")?
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .ok_or("`definitions.PlotAttributes` has no `properties` object")?;
    let mut names: Vec<String> = properties
        .keys()
        .filter(|name| is_axis_attribute_name(name))
        .cloned()
        .collect();
    names.sort();
    Ok(names)
}
