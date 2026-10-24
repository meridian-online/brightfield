//! California Housing as its authors published it, turned into a table on
//! this machine.
//!
//! The data is Pace and Barry's (1997), from the 1990 Census: one row per
//! California block group, 20,640 of them. The issuing copy is the archive
//! they gave StatLib, [`URL`], and that is what the start fetches. Brightfield
//! does not carry the data. The archive declares no licence, and StatLib's
//! submission rule guarantees free distribution and non-commercial use and
//! says nothing about commercial use, so a binary that may be used
//! commercially does not ship a copy; the reader's machine fetches one.
//!
//! The archive holds one file, `cadata.txt`: a prose header naming the
//! variables, then one row of nine whitespace-separated numbers per block
//! group. [`unpack`] reads those rows and writes them, in the order and units
//! the header gives, as a Parquet the engine opens like any other data file.
//! The nine columns are named after the header's own words ([`COLUMNS`]), and
//! no column is derived: the totals stay totals, where better-known copies of
//! this dataset divide them into per-household averages.
//!
//! What it checks is the shape, not a hash. A row that is not nine numbers,
//! or a file with no rows, is refused naming what was wrong, so a changed or
//! truncated upstream file is a banner rather than a table with the wrong
//! columns in it.

use std::io::Read;
use std::path::Path;

/// The archive the dataset's authors gave StatLib.
pub const URL: &str = "https://lib.stat.cmu.edu/datasets/houses.zip";

/// The member of [`URL`] that holds the data.
pub const MEMBER: &str = "cadata.txt";

/// The nine columns, in the order `cadata.txt` holds them, named after its
/// header: "median house value, median income, housing median age, total
/// rooms, total bedrooms, population, households, latitude, and longitude".
pub const COLUMNS: [&str; 9] = [
    "median_house_value",
    "median_income",
    "housing_median_age",
    "total_rooms",
    "total_bedrooms",
    "population",
    "households",
    "latitude",
    "longitude",
];

/// How many block groups the authors' file holds, as its header says.
pub const ROWS: usize = 20_640;

/// Read the archive at `zip` and write its rows to a Parquet at `out`.
///
/// # Errors
///
/// If the archive will not open or has no [`MEMBER`], if a row after the
/// header is not nine numbers, if there are no rows, or if DuckDB
/// cannot write the Parquet. Each names the file it was reading or writing.
pub fn unpack(zip: &Path, out: &Path) -> Result<(), String> {
    let text = read_member(zip)?;
    let rows = parse(&text).map_err(|e| format!("{} in {}: {e}", MEMBER, zip.display()))?;
    write_parquet(&rows, out)
}

/// The text of [`MEMBER`] inside the archive at `zip`.
///
/// Read as bytes and decoded leniently: the header's citation quotes the
/// paper's title in a single-byte encoding's curly quotes, which are not
/// UTF-8 and are not data.
fn read_member(zip: &Path) -> Result<String, String> {
    let file = std::fs::File::open(zip).map_err(|e| format!("{}: {e}", zip.display()))?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| format!("{} is not a zip archive: {e}", zip.display()))?;
    let mut member = archive
        .by_name(MEMBER)
        .map_err(|e| format!("{} has no {MEMBER}: {e}", zip.display()))?;
    let mut bytes = Vec::new();
    member
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{MEMBER} in {}: {e}", zip.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The rows of `cadata.txt`'s text.
///
/// The header is everything before the first line that is nine numbers. Its
/// own lines never are: the one table in it has a label and two figures per
/// line. From that line on, every non-blank line must be nine numbers.
///
/// # Errors
///
/// A line after the header that is not nine numbers, naming its line number,
/// or a text with no data rows.
pub fn parse(text: &str) -> Result<Vec<[f64; 9]>, String> {
    let mut rows = Vec::with_capacity(ROWS);
    for (number, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match row_of(line) {
            Some(row) => rows.push(row),
            None if rows.is_empty() => {}
            None => {
                return Err(format!(
                    "line {} is not nine numbers: {:?}",
                    number + 1,
                    line.trim()
                ))
            }
        }
    }
    if rows.is_empty() {
        return Err("there are no rows of nine numbers".to_string());
    }
    Ok(rows)
}

/// `line` as nine numbers, or `None` otherwise.
fn row_of(line: &str) -> Option<[f64; 9]> {
    let mut row = [0.0; 9];
    let mut fields = line.split_whitespace();
    for cell in &mut row {
        *cell = fields.next()?.parse().ok()?;
    }
    fields.next().is_none().then_some(row)
}

/// Write `rows` under [`COLUMNS`] as a Parquet at `out`, each column a
/// `DOUBLE`.
fn write_parquet(rows: &[[f64; 9]], out: &Path) -> Result<(), String> {
    let failed = |e: duckdb::Error| format!("writing {}: {e}", out.display());
    let conn = duckdb::Connection::open_in_memory().map_err(failed)?;
    let columns = COLUMNS
        .iter()
        .map(|name| format!("{name} DOUBLE"))
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute_batch(&format!("CREATE TABLE housing ({columns})"))
        .map_err(failed)?;
    {
        let mut appender = conn.appender("housing").map_err(failed)?;
        for row in rows {
            appender
                .append_row(duckdb::appender_params_from_iter(row.iter()))
                .map_err(failed)?;
        }
        appender.flush().map_err(failed)?;
    }
    conn.execute_batch(&format!(
        "COPY housing TO {} (FORMAT parquet)",
        crate::protocol_panels::sql_string(&out.to_string_lossy())
    ))
    .map_err(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The header is skipped whatever it says, and the rows after it are read
    /// in the file's order and units.
    #[test]
    fn the_rows_start_at_the_first_line_of_nine_numbers() {
        let text = "S&P Letters Data\n\
                    \t\tBols\ttols\n\
                    INTERCEPT\t\t11.4939\t275.7518\n\
                    \n\
                    \x20 4.5260000000000000e+005  8.3252000000000006e+000  4.1e+001  8.8e+002  1.29e+002  3.22e+002  1.26e+002  3.788e+001 -1.2223e+002\n\
                    \x20 3.585e+005  8.3014e+000  2.1e+001  7.099e+003  1.106e+003  2.401e+003  1.138e+003  3.786e+001 -1.2222e+002\n";
        let rows = parse(text).expect("the rows parse");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0][0], 452_600.0);
        assert_eq!(rows[0][8], -122.23);
        assert_eq!(rows[1][3], 7099.0);
    }

    /// A row that stops being nine numbers after the header is refused, naming
    /// the line, rather than skipped: a skipped row is a table that quietly
    /// holds fewer districts than its source.
    #[test]
    fn a_short_row_after_the_header_is_refused_by_line() {
        let text = "header\n1 2 3 4 5 6 7 8 9\n1 2 3 4 5 6 7 8\n";
        let refused = parse(text).expect_err("a short row is refused");
        assert!(refused.contains("line 3"), "{refused}");
    }

    /// A file that is all header is refused rather than written as an empty
    /// table.
    #[test]
    fn a_file_with_no_rows_is_refused() {
        assert!(parse("S&P Letters Data\nINTERCEPT 11.4 275.7\n").is_err());
    }
}
