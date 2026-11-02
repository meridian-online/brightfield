//! How FineType says a label's values are shown.
//!
//! Each FineType label whose cast yields a number or a date carries a display block: a d3-format
//! specifier for a number or a d3-time-format specifier for a date, plus what a specifier cannot
//! say (the value is money, in which currency when the label fixes one, a fraction shown as a
//! percentage, or a size in bytes). The blocks hold only what the label implies; scale and
//! precision are the caller's.
//!
//! The blocks are FineType's, vendored from its label definitions at the tag the rest of the app
//! stages FineType from (see `vendor/finetype/README.md`), beside the transforms the silver cast
//! applies.

use std::collections::BTreeMap;

use serde::Deserialize;

/// The text of the vendored file, which `scripts/vendor-finetype-transforms.sh` writes.
const VENDORED: &str = include_str!("../vendor/finetype/displays.json");

/// How one label's values are shown, as FineType's definition of the label says.
///
/// FineType names the two specifiers `number` and `time`; they are read into `number_format` and
/// `date_format` here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Display {
    /// The d3-format specifier for the stored number.
    #[serde(default, rename = "number")]
    pub number_format: Option<String>,
    /// The d3-time-format specifier for the stored date, time or timestamp.
    #[serde(default, rename = "time")]
    pub date_format: Option<String>,
    /// The value is an amount of money.
    #[serde(default)]
    pub money: bool,
    /// The ISO 4217 code of the one currency the label fixes, when it fixes one.
    #[serde(default)]
    pub currency: Option<String>,
    /// The stored value is a fraction, shown as a percentage.
    #[serde(default)]
    pub fraction: bool,
    /// The value is a size in bytes.
    #[serde(default)]
    pub bytes: bool,
}

/// FineType's display block for each label that has one.
#[derive(Debug, Clone, Deserialize)]
pub struct FinetypeDisplays {
    tag: String,
    labels: BTreeMap<String, Display>,
}

impl FinetypeDisplays {
    /// The display blocks vendored into this crate.
    ///
    /// # Panics
    ///
    /// If the vendored file does not parse, which `vendored_displays_read` in the tests rules out.
    #[must_use]
    pub fn vendored() -> Self {
        Self::from_json(VENDORED).expect("the vendored displays parse")
    }

    /// Read display blocks from the JSON `scripts/vendor-finetype-transforms.sh` writes.
    ///
    /// # Errors
    ///
    /// The parse error, when `json` is not that shape.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// The FineType tag the blocks were taken from.
    #[must_use]
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// The display block of `label`, or `None` for a label without a block, such as a text label
    /// (`vendored_displays_read` reads one), or a label FineType does not define.
    #[must_use]
    pub fn get(&self, label: &str) -> Option<&Display> {
        self.labels.get(label)
    }

    /// Every label with a display block and its block, in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Display)> {
        self.labels.iter().map(|(label, d)| (label.as_str(), d))
    }
}

#[cfg(test)]
mod tests {
    use brightfield_spec::date_format::DateFormat;
    use brightfield_spec::number_format::NumberFormat;

    use super::*;
    use crate::silver::FinetypeTransforms;

    #[test]
    fn vendored_displays_read() {
        let d = FinetypeDisplays::vendored();
        assert_eq!(
            d.get("datetime.component.year")
                .and_then(|b| b.number_format.as_deref()),
            Some("d")
        );
        assert!(d.get("finance.currency.amount").is_some_and(|b| b.money));
        assert!(d
            .get("representation.numeric.percentage")
            .is_some_and(|b| b.fraction));
        assert!(d
            .get("representation.file.file_size")
            .is_some_and(|b| b.bytes));
        assert_eq!(d.get("representation.text.word"), None);
    }

    #[test]
    fn vendored_displays_share_the_transforms_tag() {
        assert_eq!(
            FinetypeDisplays::vendored().tag(),
            FinetypeTransforms::vendored().tag()
        );
    }

    #[test]
    fn every_number_format_parses() {
        let d = FinetypeDisplays::vendored();
        let mut numbers = 0;
        let mut dates = 0;
        for (label, block) in d.iter() {
            assert!(
                block.number_format.is_some() != block.date_format.is_some(),
                "{label}: a block carries exactly one specifier"
            );
            if let Some(spec) = &block.number_format {
                assert!(
                    NumberFormat::parse(spec).is_some(),
                    "{label}: number format {spec:?} does not parse"
                );
                numbers += 1;
            }
            if let Some(spec) = &block.date_format {
                if let Err(e) = DateFormat::parse(spec) {
                    panic!("{label}: date format {spec:?} does not parse: {e}");
                }
                dates += 1;
            }
        }
        assert!(
            numbers > 0 && dates > 0,
            "the vendored file holds number and date blocks"
        );
    }

    #[test]
    fn a_block_reads_finetype_key_names() {
        let d = FinetypeDisplays::from_json(
            r#"{"tag":"v0","source":"","labels":{"x.y.z":{"number":"(,.2f","money":true,"currency":"CHF"}}}"#,
        )
        .unwrap();
        assert_eq!(
            d.get("x.y.z"),
            Some(&Display {
                number_format: Some("(,.2f".into()),
                money: true,
                currency: Some("CHF".into()),
                ..Display::default()
            })
        );
    }
}
