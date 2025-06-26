use chrono::{Days, NaiveDate};
use clap::Parser;
use duckdb::{AccessMode, Config, Connection, Error};
use std::collections::HashSet;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process;
use tqdm::tqdm;
mod helpers;
use helpers::handle_output_path;
/// Command line arguments
#[derive(Parser, Debug)]
#[command(author, version, about = "Copy DICOM files based on patient IDs", long_about = None)]

struct Opt {
    /// Whether to overwrite existing files
    #[arg(short, long)]
    overwrite: bool,

    /// File containing patient IDs
    #[arg(name = "PATIENT_ID_FILE")]
    patient_id_file: String,

    /// Directory to store copied files
    #[arg(name = "OUTPUT_DIRECTORY")]
    output_directory: String,

    /// Database file to use
    #[arg(short = 'd', long = "database", default_value = "open_sight.duckdb")]
    database: String,
}

fn read_patient_ids(file_path: &str) -> Result<Vec<String>, std::io::Error> {
    let contents = fs::read_to_string(file_path)?;
    Ok(contents
        .lines()
        .map(|line| line.trim().to_string())
        .collect())
}

fn copy_files(
    patient_id: &str,
    output_directory: &str,
    overwrite: bool,
    conn: &Connection,
) -> Result<bool, Error> {
    let query = format!( "SELECT * FROM main.open_sight WHERE patient_id = '{}' AND modality IN ('OP','OPT') AND manufacturer = 'Heidelberg Engineering' ORDER BY patient_id, scan_date, laterality, modality", patient_id );
    // 0          1            2          3   4   5         6        7            8                  9        10        11
    // patient_id,patient_name,laterality,sex,dob,scan_date,modality,manufacturer,series_description,modified,file_size,file_path

    let mut stmt = conn.prepare(&query)?;
    let rows: Vec<_> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(2)?,  // laterality
                row.get::<_, u64>(5)?,     // scan_date
                row.get::<_, String>(6)?,  // modality
                row.get::<_, String>(11)?, // file_path
            ))
        })?
        .filter_map(|result| result.ok())
        .collect();

    // If rows are empty, the patient ID was not found in the database
    if rows.is_empty() {
        return Ok(false);
    }

    let mut missing_files = HashSet::new();

    for (laterality, scan_date_days, modality, file_path) in tqdm(rows) {
        let scan_date = NaiveDate::from_ymd_opt(1970, 1, 1)
            .unwrap()
            .checked_add_days(Days::new(scan_date_days))
            .unwrap();
        let file_name = Path::new(&file_path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let output_file_name = format!("{}_{}", modality, file_name);
        let formatted_date = scan_date.format("%Y%m%d").to_string();

        // Create the output directory structure
        let patient_folder = Path::new(output_directory)
            .join(patient_id)
            .join(format!("{}_{}", formatted_date, laterality));

        if !patient_folder.exists() {
            fs::create_dir_all(&patient_folder).unwrap();
        }

        let output_file_path = patient_folder.join(output_file_name);
        if (!output_file_path.exists() || overwrite)
            && fs::copy(&file_path, &output_file_path).is_err()
        {
            missing_files.insert(file_path);
        }
    }

    Ok(missing_files.is_empty())
}

fn main() {
    let args = Opt::parse();
    let patient_ids = read_patient_ids(&args.patient_id_file).unwrap_or_else(|err| {
        eprintln!("Error reading patient ID file: {}", err);
        process::exit(1);
    });

    let config = Config::default()
        .access_mode(AccessMode::ReadOnly)
        .unwrap_or_else(|err| {
            eprintln!("Error setting access mode: {}", err);
            process::exit(1);
        });
    let conn = Connection::open_with_flags(&args.database, config).unwrap_or_else(|err| {
        eprintln!("Error connecting to database: {}", err);
        process::exit(1);
    });

    let mut not_found_patients = Vec::new();
    for patient_id in tqdm(&patient_ids) {
        match copy_files(patient_id, &args.output_directory, args.overwrite, &conn) {
            Ok(false) => not_found_patients.push(patient_id.clone()),
            Err(e) => {
                eprintln!("Error processing patient {}: {}", patient_id, e);
                process::exit(1);
            }
            _ => {}
        }
    }

    if !not_found_patients.is_empty() {
        let output_path = PathBuf::from("patient_ids_not_found.csv");

        // Handle the output file path, pass `true` for overwriting the file
        let _ = handle_output_path(&output_path, args.overwrite);

        // Open the file in write mode, handling the Result
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(&output_path)
            .unwrap_or_else(|err| {
                eprintln!("Error opening output file: {}", err);
                process::exit(1);
            });

        // Write the patient IDs to the file
        for patient_id in not_found_patients {
            writeln!(file, "{}", patient_id).unwrap_or_else(|err| {
                eprintln!("Error writing to output file: {}", err);
                process::exit(1);
            });
        }
        println!(
            "Patient IDs not found in DB, see file: {}",
            output_path.display()
        );
    }
}
