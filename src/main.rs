use chrono::{DateTime, Datelike, Local, NaiveDate};
use clap::Parser;
use dicom_dictionary_std::tags;
use dicom_object::OpenFileOptions;
use rayon::prelude::*;
use serde::Deserialize;
use serde::Serialize;
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::BufReader;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};
use std::{env, fs, io};
use std::{process::Command, thread, time::Duration};
use sysinfo::System;
use tempfile::tempdir;
use walkdir::WalkDir;
mod helpers;
use helpers::handle_output_path;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(required = true)]
    folder_paths: Vec<PathBuf>,

    #[arg(short, long, default_value = "open_sight_results.csv")]
    csv_out: String,

    #[arg(short, long, default_value_t = 1)]
    num_jobs: usize,

    #[arg(short, long, help = "Don't append existing CSV, overwriting it")]
    overwrite: bool,

    #[arg(short, long, default_value_t = 50)]
    batch_size: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct DicomData {
    patient_id: String,
    patient_name: String,
    laterality: String,
    sex: String,
    dob: String,
    scan_date: String,
    modality: String,
    manufacturer: String,
    series_description: String,
    modified: String,
    file_size: u64,
    file_path: String,
}

#[derive(Deserialize, Debug)]
struct CEMetadata {
    patient: PatientData,
    exam: ExamData,
    series: SeriesData,
}

#[derive(Deserialize, Debug)]
struct PatientData {
    patient_key: Option<String>,
    first_name: Option<String>,
    last_name: Option<String>,
    date_of_birth: Option<String>,
    gender: Option<String>,
}

#[derive(Deserialize, Debug)]
struct ExamData {
    manufacturer: Option<String>,
    scan_datetime: Option<String>,
}

#[derive(Deserialize, Debug)]
struct SeriesData {
    laterality: Option<String>,
    protocol: Option<String>,
}

const CE_EXT: &[&str] = &["e2e", "fda", "sdb", "dcm"];
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Start measuring time
    let start_time = Instant::now();

    // Get the initial CPU usage.
    let mut system = System::new_all();
    system.refresh_all();

    // Parse command line arguments using Clap
    let args = Args::parse();

    // Use the parsed arguments
    let folder_paths = args.folder_paths;
    let csv_out = &args.csv_out;
    let num_jobs = args.num_jobs;
    let overwrite = args.overwrite;
    let batch_size = args.batch_size;

    // Get crystal-eye path from environment variable or default to "./crystal-eye"
    let mut crystal_eye_path =
        env::var("CRYSTAL_EYE_PATH").unwrap_or_else(|_| "crystal-eye".to_string());

    // Check if the crystal-eye binary exists
    check_crystal_eye_path(&mut crystal_eye_path);

    // Number of CPUs:
    println!(
        ">> Using {} of {} CPUs possible",
        num_jobs,
        system.cpus().len()
    );

    // Check if the CSV file exists and rename it if necessary
    let output_path = PathBuf::from(csv_out);

    let mut processed_file_paths = HashSet::new();
    if output_path.exists() && !overwrite {
        processed_file_paths = read_existing_csv(&output_path)?;
    } else {
        handle_output_path(&output_path, overwrite)?;
    }

    if let Ok(current_dir) = env::current_dir() {
        let full_path = current_dir.join(&output_path);
        println!(">> Saving results to CSV file: {:?}", full_path);
    } else {
        println!("!! Error getting current working directory");
    }

    let mut timenow = Instant::now();
    let mut counter = 0;

    // Iterate over each matched folder and process the files
    for folder_path in folder_paths {
        let mut input_files: Vec<PathBuf> = Vec::new();

        if folder_path.is_dir() {
            println!(
                ">> Walking directory and processing files in {:?}",
                folder_path
            );
        }

        for entry in WalkDir::new(&folder_path) {
            match entry {
                Ok(entry) => {
                    if entry.path().extension().map_or(false, |ext| {
                        CE_EXT
                            .iter()
                            .any(|ext_pattern| ext.eq_ignore_ascii_case(ext_pattern))
                    }) {
                        if entry
                            .path()
                            .metadata()
                            .map_or(false, |meta| meta.len() == 0)
                        {
                            eprintln!("ERROR: Empty file: {:?}", entry.path());
                            continue;
                        }
                        input_files.push(entry.path().to_path_buf());
                        counter += 1;
                    }

                    if input_files.len() >= batch_size {
                        if let Err(err) = process_and_save_results(
                            &input_files,
                            &output_path,
                            num_jobs,
                            &crystal_eye_path,
                            &processed_file_paths,
                        ) {
                            eprintln!("ERROR: {:?}, reason: {:?}", &input_files, err);
                        }

                        input_files.clear();
                        print_speed(&timenow, batch_size as f32, counter);
                        timenow = Instant::now();
                    }
                }
                Err(err) => {
                    // Handle the error, e.g., log the error and continue
                    eprintln!("Error: {:?}", err);
                }
            }
        }

        if !input_files.is_empty() {
            if let Err(err) = process_and_save_results(
                &input_files,
                &output_path,
                num_jobs,
                &crystal_eye_path,
                &processed_file_paths,
            ) {
                eprintln!("ERROR: {:?}, reason: {:?}", &input_files, err);
            }
            print_speed(&timenow, input_files.len() as f32, counter);
            println!()
        }
    }

    if output_path.exists() {
        println!(
            ">> Results saved to {:?}",
            output_path.canonicalize().unwrap()
        );
    } else {
        println!(">> No data to save. Skipping CSV file creation.");
    }

    let tot_time = start_time.elapsed();
    println!(
        ">> processed: {} | Time elapsed: {:.2?} | Avg. speed: {:.2} it/s",
        counter,
        tot_time,
        counter as f32 / tot_time.as_secs_f32()
    );

    Ok(())
}

fn read_existing_csv(csv_path: &Path) -> Result<HashSet<String>, Box<dyn std::error::Error>> {
    let mut file_paths = HashSet::new();

    let file = File::open(csv_path)?;
    let mut rdr = csv::Reader::from_reader(BufReader::new(file));

    for result in rdr.deserialize() {
        let record: DicomData = result?;
        file_paths.insert(record.file_path);
    }
    Ok(file_paths)
}

fn check_crystal_eye_path(crystal_eye_path: &mut String) {
    let path = Path::new(crystal_eye_path);

    if path.exists() {
        println!(">> crystal-eye found at: {}", path.display());
        return;
    }

    if let Ok(path_var) = std::env::var("PATH") {
        for path in std::env::split_paths(&path_var) {
            let full_path = path.join(&*crystal_eye_path);
            if full_path.exists() {
                println!(">> crystal-eye found at: {}", full_path.display());
                *crystal_eye_path = full_path.to_string_lossy().into_owned(); // Update the path
                return;
            }
        }
    }

    eprintln!(
        ">> WARNING: crystal-eye not found at: {}\n   Only DICOM files will be processed, if any\n   Use 'export CRYSTAL_EYE_PATH=_path_to_crystal-eye_'",
        crystal_eye_path
    );
    *crystal_eye_path = String::new();
}

fn print_speed(start_time: &Instant, iterations: f32, counter: i32) {
    print!(
        "\r>> Speed: {:.2} it/s, {} DCMs processed",
        iterations / start_time.elapsed().as_secs_f32(),
        counter
    );
    std::io::stdout().flush().unwrap();
}

fn process_and_save_results(
    input_files: &[PathBuf],
    output_path: &Path,
    num_jobs: usize,
    crystal_eye_path: &str,
    processed_file_paths: &HashSet<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Process DICOM files in parallel
    let results: Vec<_> = input_files
        .par_chunks(num_jobs)
        .map(|chunk| process_input_files(chunk, crystal_eye_path, processed_file_paths))
        .flatten()
        .collect();

    if !results.is_empty() {
        save_results_to_csv(&results, output_path)?;
    }

    Ok(())
}

fn save_results_to_csv(
    results: &[DicomData],
    output_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let file: File = OpenOptions::new()
        .create(true)
        .append(true)
        .open(output_path)?;
    let mut wtr = csv::WriterBuilder::new()
        .has_headers(false)
        .from_writer(file);
    // Check if the file is empty (has no content) before writing the header
    if output_path.metadata()?.len() == 0 {
        // Write the header only if the file is empty
        wtr.write_record([
            "patient_id",
            "patient_name",
            "laterality",
            "sex",
            "dob",
            "scan_date",
            "modality",
            "manufacturer",
            "series_description",
            "modified",
            "file_size",
            "file_path",
        ])?;
    }
    for result in results {
        wtr.serialize(result)?;
    }
    wtr.flush()?;
    Ok(())
}

fn process_input_files(
    paths: &[PathBuf],
    crystal_eye_path: &str,
    existing_paths: &HashSet<String>,
) -> Vec<DicomData> {
    paths
        .iter()
        .filter(|path| path.metadata().ok().map_or(false, |m| m.len() > 0))
        .filter_map(|path| {
            let absolute_path = match path.canonicalize() {
                Ok(abs_path) => abs_path,
                Err(e) => {
                    eprintln!("Error obtaining canonical path for {:?}: {}", path, e);
                    return None;
                }
            };
            if existing_paths.contains(absolute_path.to_str().unwrap_or_default()) {
                return None; // Skip already processed files
            }
            if let Some(ext) = path.extension() {
                if ext.eq_ignore_ascii_case("dcm") {
                    match extract_dicom_data_with_retry(path, 10) {
                        Ok(data) => Some(data),
                        Err(e) => {
                            eprintln!("Error processing DCM input file {:?}: {}", path, e);
                            None
                        }
                    }
                } else if CE_EXT
                    .iter()
                    .any(|ext_pattern| ext.eq_ignore_ascii_case(ext_pattern))
                    && !crystal_eye_path.is_empty()
                {
                    match extract_crystal_eye_data(path, crystal_eye_path) {
                        Ok(data) => Some(data),
                        Err(e) => {
                            eprintln!("Error processing crystal-eye input file {:?}: {}", path, e);
                            None
                        }
                    }
                } else {
                    None // Skip files with other extensions
                }
            } else {
                None // Skip files without extensions
            }
        })
        .collect()
}

fn extract_crystal_eye_data(
    path: &Path,
    crystal_eye_path: &str,
) -> Result<DicomData, Box<dyn std::error::Error>> {
    let temp_dir = tempdir()?;
    let output_dir = temp_dir.path();

    // Convert path to string and replace backslashes with forward slashes
    let path_str = path.to_string_lossy().replace('\\', "/");

    // Run crystal-eye command
    let output = Command::new(crystal_eye_path)
        .arg("-i")
        .arg(&path_str)
        .arg("--only-metadata")
        .arg("-o")
        .arg(output_dir)
        .output()?;

    if !output.status.success() {
        return Err(format!("crystal-eye command failed with status: {}", output.status).into());
    }

    // Read metadata.json
    let metadata_path = output_dir.join("metadata.json");
    let metadata_file = File::open(metadata_path)?;
    let metadata: CEMetadata = serde_json::from_reader(metadata_file)?;

    // Use unwrap_or("") to handle null values and replace them with empty strings.
    let patient_name = format!(
        "{} {}",
        metadata.patient.first_name.unwrap_or_default(),
        metadata.patient.last_name.unwrap_or_default()
    );

    let formatted_patient_dob = format_date(
        &metadata.patient.date_of_birth.unwrap_or_default(),
        Some("%Y-%m-%d"),
    );
    let formatted_content_date = format_date(
        &metadata.exam.scan_datetime.unwrap_or_default(),
        Some("%Y-%m-%d %H:%M:%S%.f"),
    );

    let file_path = path
        .canonicalize()?
        .to_str()
        .ok_or("Invalid file path")?
        .to_string();

    let ce_file = fs::metadata(path)?;
    let modified = format_modified_datetime(ce_file.modified());
    let file_size = ce_file.len();

    Ok(DicomData {
        patient_id: metadata.patient.patient_key.unwrap_or_default(),
        patient_name,
        laterality: metadata.series.laterality.unwrap_or_default(),
        sex: metadata.patient.gender.unwrap_or_default(),
        dob: formatted_patient_dob,
        scan_date: formatted_content_date,
        modality: "CE".to_string(),
        manufacturer: metadata.exam.manufacturer.unwrap_or_default(),
        series_description: metadata.series.protocol.unwrap_or_default(),
        modified,
        file_size,
        file_path,
    })
}

fn extract_dicom_data_with_retry(
    path: &Path,
    max_retries: u64,
) -> Result<DicomData, Box<dyn std::error::Error>> {
    let mut retries = 0;
    loop {
        match extract_dicom_data(path) {
            Ok(data) => return Ok(data),
            Err(e) if retries < max_retries => {
                eprintln!("Error processing {:?}: {}. Retrying...", path, e);
                retries += 1;
                thread::sleep(Duration::from_millis(500 * retries));
            }
            Err(e) => return Err(e),
        }
    }
}

fn extract_dicom_data(path: &Path) -> Result<DicomData, Box<dyn std::error::Error>> {
    let obj = OpenFileOptions::new()
        .read_until(tags::PIXEL_DATA)
        .open_file(path)?;

    let patient_id = if let Some(elem) = obj.element_opt(tags::PATIENT_ID)? {
        elem.to_str()?
    } else {
        "".into()
    };
    let patient_name = if let Some(elem) = obj.element_opt(tags::PATIENT_NAME)? {
        elem.to_str()?
    } else {
        "".into()
    };
    let image_laterality = if let Some(elem) = obj.element_opt(tags::IMAGE_LATERALITY)? {
        elem.to_str()?
    } else if let Some(elem) = obj.element_opt(tags::LATERALITY)? {
        elem.to_str()?
    } else {
        "".into()
    };
    let patient_sex = if let Some(elem) = obj.element_opt(tags::PATIENT_SEX)? {
        elem.to_str()?
    } else {
        "".into()
    };
    let patient_dob = if let Some(elem) = obj.element_opt(tags::PATIENT_BIRTH_DATE)? {
        elem.to_str()?
    } else {
        "".into()
    };
    let content_date = if let Some(elem) = obj.element_opt(tags::CONTENT_DATE)? {
        elem.to_str()?
    } else {
        "".into()
    };
    let modality = if let Some(elem) = obj.element_opt(tags::MODALITY)? {
        elem.to_str()?
    } else {
        "".into()
    };
    let manufacturer = if let Some(elem) = obj.element_opt(tags::MANUFACTURER)? {
        elem.to_str()?
    } else {
        "".into()
    };
    let series_description = if let Some(elem) = obj.element_opt(tags::SERIES_DESCRIPTION)? {
        elem.to_str()?
    } else {
        "".into()
    };

    let formatted_patient_dob = format_date(&patient_dob, None);
    let formatted_content_date = format_date(&content_date, None);

    // let file_path = path::absolute(path)?.to_string_lossy().to_string();
    let file_path = path
        .canonicalize()?
        .to_str()
        .ok_or("Invalid file path")?
        .to_string();

    let metadata = fs::metadata(path)?;

    let modified = format_modified_datetime(metadata.modified());
    let file_size = metadata.len();

    // patient_id,patient_name,laterality,sex,dob,scan_date,modality,manufacturer,series_description,modified,file_size,file_path
    Ok(DicomData {
        patient_id: patient_id.to_string(),
        patient_name: patient_name.to_string(),
        laterality: image_laterality.to_string(),
        sex: patient_sex.to_string(),
        dob: formatted_patient_dob,
        scan_date: formatted_content_date,
        modality: modality.to_string(),
        manufacturer: manufacturer.to_string(),
        series_description: series_description.to_string(),
        modified,
        file_size,
        file_path,
    })
}

fn format_modified_datetime(modified: io::Result<SystemTime>) -> String {
    match modified {
        Ok(time) => {
            // Convert SystemTime to DateTime<Local>
            let datetime: DateTime<Local> = time.into();
            // Format the datetime with the specified format
            datetime.format("%d-%m-%Y %H:%M:%S").to_string()
        }
        Err(_) => {
            // Return an empty string if there's an error
            "".to_string()
        }
    }
}

fn format_date(date_str: &str, format_str: Option<&str>) -> String {
    let default_format = "%Y%m%d";
    let format_to_use = format_str.unwrap_or(default_format);

    if let Ok(parsed_date) = NaiveDate::parse_from_str(date_str, format_to_use) {
        parsed_date.format("%d-%m-%Y").to_string()
    } else if format_str.is_none() {
        // Attempt to handle ambiguous dates
        attempt_ambiguous_date_parse(date_str)
    } else {
        String::new()
    }
}

fn attempt_ambiguous_date_parse(date_str: &str) -> String {
    // Try parsing assuming no century (e.g., "010180" becomes "1980-01-01")
    if let Ok(parsed_date) = NaiveDate::parse_from_str(date_str, "%y%m%d") {
        // Check if the parsed year is within a reasonable range
        if parsed_date.year() > 1900 {
            // Adjust threshold as needed
            return parsed_date.format("%d-%m-%Y").to_string();
        }
    }

    String::new() // Return empty if still unsuccessful
}
