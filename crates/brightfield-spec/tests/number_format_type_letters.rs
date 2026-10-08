//! **A d3-format specifier's type is one the format names, or the editor can
//! tell it is not**: [`NumberFormat::parse`] reads any letter in the type's
//! place, as d3-format does, so a hand-edited file still draws, and
//! [`NumberFormat::names_its_type`] is the judgement a field that keeps a
//! specifier makes over the same specifier.

use brightfield_spec::number_format::NumberFormat;

/// The letters d3-format names a type by, written out here and not read from the
/// crate, so a letter added to or dropped from the crate's list is a test that
/// reddens.
const NAMED: [char; 14] = [
    '%', 'b', 'c', 'd', 'e', 'f', 'g', 'n', 'o', 'p', 'r', 's', 'X', 'x',
];

/// Every letter an ASCII specifier can end in: the letters, and `%`.
fn every_type_position() -> Vec<char> {
    ('a'..='z').chain('A'..='Z').chain(['%']).collect()
}

/// **Exactly the fourteen letters d3-format names are held to name their type**,
/// with a precision before them and without one, and each other letter is not.
/// The reader takes every one of them either way, since a letter that names no
/// type is `.12~g` there.
#[test]
fn the_editor_names_the_types_d3_format_names_and_the_reader_takes_every_letter() {
    for letter in every_type_position() {
        for specifier in [format!("{letter}"), format!(".2{letter}")] {
            let parsed = NumberFormat::parse(&specifier)
                .unwrap_or_else(|| panic!("the reader refused `{specifier}`"));
            assert_eq!(
                parsed.names_its_type(),
                NAMED.contains(&letter),
                "`{specifier}`: does its type name a format"
            );
        }
    }
}

/// **A specifier with no type at all is not a typo**: `,`, `.2` and the empty
/// specifier name no type, and the editor takes them as the reader does.
#[test]
fn a_specifier_with_no_type_names_none_and_is_not_refused_for_it() {
    for specifier in ["", ",", ".2", "+.1", "$,.2", "~"] {
        let parsed = NumberFormat::parse(specifier)
            .unwrap_or_else(|| panic!("the reader refused `{specifier}`"));
        assert!(
            parsed.names_its_type(),
            "`{specifier}` has no type to refuse"
        );
    }
}

/// **The list the refusal prints is the list the judgement keeps**: the crate's
/// letters are the fourteen, in the order a sentence can name them.
#[test]
fn the_type_letters_the_crate_prints_are_the_ones_it_accepts() {
    let printed: Vec<char> = NumberFormat::TYPE_LETTERS.chars().collect();
    let mut sorted_printed = printed.clone();
    sorted_printed.sort_unstable();
    let mut sorted_named = NAMED.to_vec();
    sorted_named.sort_unstable();
    assert_eq!(sorted_printed, sorted_named);
    for letter in printed {
        let parsed = NumberFormat::parse(&format!(".1{letter}")).expect("the reader takes it");
        assert!(parsed.names_its_type(), "`{letter}` is printed as a type");
    }
}
