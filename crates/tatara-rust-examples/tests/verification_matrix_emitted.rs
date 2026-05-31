/// Emit ONE aggregate `#[test] fn $name` that runs `$run` against every
/// row of `$rows`, collecting EVERY failure before asserting — so one
/// run reports all broken rows, not just the first.
///
/// `$run` is any `Fn(&Row) -> Result<(), E>` where `E: Display` and
/// `Row: Debug`. The row table is anything with `.iter()` + `.len()`
/// (`&[Row]`, `Vec<Row>`, an array).
#[macro_export]
macro_rules! verification_matrix {
    ($(#[$meta:meta])* name = $name:ident, rows = $rows:expr, run = $run:expr $(,)?) => {
        $(#[$meta])* #[test] fn $name () { let __vm_run = $run; let __vm_rows = $rows;
        let mut __vm_failures : ::std::vec::Vec <::std::string::String > =
        ::std::vec::Vec::new(); for __vm_row in __vm_rows.iter() { if let
        ::std::result::Result::Err(__vm_e) = __vm_run(__vm_row) { __vm_failures
        .push(::std::format!("{:?}: {}", __vm_row, __vm_e)); } }
        ::std::assert!(__vm_failures.is_empty(), "{}/{} matrix rows failed:\n  - {}",
        __vm_failures.len(), __vm_rows.len(), __vm_failures.join("\n  - "),); }
    };
}
/// Emit a `#[test] fn $name` build-gate asserting the row table covers
/// exactly `$count` cases — typically an enum's `pleme-variantcount-derive`
/// `COUNT`. A new variant (or dispatch arm) landing without a matrix row
/// trips this test: the forcing function.
#[macro_export]
macro_rules! matrix_covers_all {
    (
        $(#[$meta:meta])* name = $name:ident, rows = $rows:expr, covers = $count:expr
        $(,)?
    ) => {
        $(#[$meta])* #[test] fn $name () { let __vm_rows = $rows;
        ::std::assert_eq!(__vm_rows.len(), $count,
        "matrix `{}` must cover all {} cases; got {} rows (a new variant/arm landed without a matrix row)",
        ::std::stringify!($name), $count, __vm_rows.len(),); }
    };
}
