//! The name a step made from typed SQL gets: the table it reads and the one column it turns on.

use brightfield_shell::typed_sql::typed_step_name;

fn named(sql: &str, taken: &[&str]) -> String {
    let reading = brightfield_engine::statement_reading(sql).expect("DuckDB reads it");
    typed_step_name(&reading, &taken.iter().map(|t| t.to_string()).collect())
}

#[test]
fn a_group_by_of_one_column_names_the_step() {
    assert_eq!(
        named(
            "select house_age, avg(median_income) from california_housing group by house_age",
            &[]
        ),
        "california_housing_sql_by_house_age"
    );
    assert_eq!(
        named("select house_age, count(*) from california_housing where median_income > 3 group by house_age", &[]),
        "california_housing_sql_by_house_age"
    );
}

#[test]
fn a_window_partitioned_by_one_column_names_the_step() {
    assert_eq!(
        named("select *, rank() over (partition by house_age order by median_income) from california_housing", &[]),
        "california_housing_sql_by_house_age"
    );
}

#[test]
fn a_where_naming_one_column_names_the_step() {
    assert_eq!(
        named(
            "select median_income from california_housing where house_age > 28",
            &[]
        ),
        "california_housing_sql_by_house_age"
    );
    assert_eq!(
        named(
            "select * from california_housing as t where t.house_age > 28 or HOUSE_AGE < 5",
            &[]
        ),
        "california_housing_sql_by_house_age"
    );
}

#[test]
fn a_statement_turning_on_no_one_column_is_named_by_its_table() {
    for sql in [
        "select avg(median_income) from california_housing",
        "select house_age, latitude, count(*) from california_housing group by house_age, latitude",
        "select * from california_housing where house_age > 28 and median_income > 3",
        "select house_age, count(*) from california_housing where median_income > 3 group by all",
        "select * from california_housing union all select * from california_housing where house_age > 1",
    ] {
        assert_eq!(named(sql, &[]), "california_housing_sql", "{sql}");
    }
}

#[test]
fn a_taken_name_gets_the_next_number() {
    let sql = "select avg(median_income) from california_housing";
    assert_eq!(
        named(sql, &["CALIFORNIA_HOUSING_SQL"]),
        "california_housing_sql_2"
    );
    assert_eq!(
        named(sql, &["california_housing_sql", "california_housing_sql_2"]),
        "california_housing_sql_3"
    );
}

#[test]
fn the_tables_read_leave_out_ctes_and_table_functions() {
    let r = brightfield_engine::statement_reading(
        "with a as (select * from t1) select * from a join t2 using (k), range(3)",
    )
    .unwrap();
    assert_eq!(r.tables, vec!["t1".to_string(), "t2".to_string()]);
}

#[test]
fn the_table_named_is_the_first_in_the_text_even_when_a_subquery_sits_in_the_join() {
    let r = brightfield_engine::statement_reading(
        "select * from a join b on a.k = (select max(k) from c)",
    )
    .unwrap();
    assert_eq!(
        r.tables,
        vec!["a".to_string(), "b".to_string(), "c".to_string()]
    );
}
