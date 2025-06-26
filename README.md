# OpenSight

A parallel DICOM crawler for extracting specific metadata (such as patient ID, image laterality, etc.) and writing the results to a CSV file

## Usage

```bash
A CLI tool for crawling DICOM and Crystal-Eye files and extracting metadata to a CSV file

Usage: open-sight [OPTIONS] <FOLDER_PATHS>...

Arguments:
  <FOLDER_PATHS>...

Options:
  -c, --csv-out <CSV_OUT>        [default: open_sight_results.csv]
  -n, --num-jobs <NUM_JOBS>      [default: 1]
  -o, --overwrite
  -b, --batch-size <BATCH_SIZE>  [default: 50]
  -h, --help                     Print help
  -V, --version                  Print version
```

```bash
Copy DICOM files and Crystal-Eye files based on patient IDs

Usage: copy_src [OPTIONS] <PATIENT_ID_FILE> <OUTPUT_DIRECTORY>

Arguments:
  <PATIENT_ID_FILE>   File containing patient IDs
  <OUTPUT_DIRECTORY>  Directory to store copied files

Options:
  -o, --overwrite            Whether to overwrite existing files
  -d, --database <DATABASE>  Database file to use [default: open_sight.duckdb]
  -h, --help                 Print help
```

## Converting CSV to duckdb

Run in a terminal:

```bash
duckdb open_sight.duckdb
```

Then run these commands in the `duckdb` terminal, assuming `all.csv` was the file created by `open-sight`:

```sql
CREATE TABLE open_sight (
    patient_id VARCHAR,
    patient_name VARCHAR,
    laterality VARCHAR,
    sex VARCHAR,
    dob DATE,
    scan_date DATE,
    modality VARCHAR,
    manufacturer VARCHAR,
    series_description VARCHAR,
    modified TIMESTAMP,
    file_size BIGINT,
    file_path VARCHAR PRIMARY KEY
);

CREATE UNIQUE INDEX idx_file_path ON open_sight ("file_path");

INSERT INTO open_sight
SELECT DISTINCT *
FROM read_csv_auto('all.csv') AS csv
WHERE NOT EXISTS (
    SELECT 1
    FROM open_sight
    WHERE open_sight.file_path = csv.file_path
);
```

If just updating the DB, just run:

```sql
INSERT INTO open_sight
SELECT DISTINCT *
FROM read_csv_auto('all.csv') AS csv
WHERE NOT EXISTS (
    SELECT 1
    FROM open_sight
    WHERE open_sight.file_path = csv.file_path
);

-- To get the new totals
select count(*) from open_sight;

-- Some basic table analysis
SELECT * FROM information_schema.tables WHERE table_schema = 'main';
SELECT * FROM duckdb_indexes();
SELECT * FROM duckdb_constraints();
SELECT * FROM duckdb_tables();
```

## Usage Examples

### Crawling DICOM (or proprietary files if `crystal-eye` is present) files and saving results to a CSV file

- `_input_folder_`: a folder containing DICOM files in no matter folder structure, with subfolders etc.
- `_csv_file_`: a CSV file where the results will be saved; if given a previous populated one, data already parsed will be skipped.

```bash
open-sight _input_folder_/* -c _csv_file_ 2>&1 | tee output.log
```

### Copy any files in `file_path` column based on patient IDs using the Database

- `patient_ids.txt`: a simple file containing the patient_ids in rows.
- `_output_folder_`: the folder where the files will be copied.

```bash
copy_src patient_ids.txt /_output_folder_ -d open_sight.duckdb
```

## Bumping Version

Bump the version number by running `cargo v [part]` where `[part]` is `major`, `minor`, or `patch`, depending on which part of the version number you want to bump.

```bash
cargo install cargo-v

# commit
cargo v patch -y #
# push
cargo build --release -j 10
git push origin --tags
```

## Changelog

- 0.3.3
  - Renamed to `copy_src` and `copy_src_csv`
- 0.3.2
  - Updated `copy_dcms` to use updated database format
- 0.3.1
  - Fixed a bug where DCM need to be checked first, then use `crystal-eye`
- 0.3.0
  - Updated `duckdb` to `v1.0.0`
  - Ability to reuse the CSV to skip already processed files
- 0.2.1
  - Extend support to all extensions handled by `crystal-eye`: `e2e`, `fda` and `sdb`
- 0.2.0
  - Added `E2E` support via `crystal-eye`
- 0.1.6
  - Added `copy_dcms` to replace `find_patid` and `copy_dcms_csv`
- 0.1.5
  - Changed `file_size` to u64 type and representing `bytes`
- 0.1.4
  - Renamed the table headers to lowercase with underscore instead of spaces
- 0.1.3
  - Introduced `find_patid`
  - Refactored code to use `helpers.rs`
- 0.1.2
  - Reverted `path::absolute`, keep Windows file path way
- 0.1.1
  - Able to use glob
  - Retry routine for failed DCM during parsing
  - Using experimental `path::absolute` to properly render Windows full path strings
