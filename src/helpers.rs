use std::fs;
use std::fs::remove_file;
use std::path::PathBuf;

pub fn handle_output_path(output_path: &PathBuf, overwrite: bool) -> std::io::Result<()> {
    // Check if the CSV file exists and rename it if necessary
    if output_path.exists() {
        if overwrite {
            println!(">> Overwriting existing file: {:?}", output_path);
            remove_file(output_path.clone())?;
        } else {
            let mut counter = 1;
            let mut new_path = output_path.with_file_name(format!(
                "{}_{}.csv",
                output_path.file_stem().unwrap().to_str().unwrap(),
                counter
            ));
            while new_path.exists() {
                counter += 1;
                new_path = output_path.with_file_name(format!(
                    "{}_{}.csv",
                    output_path.file_stem().unwrap().to_str().unwrap(),
                    counter
                ));
            }
            println!(">> Moved old {:?} to {:?}", &output_path, &new_path);
            fs::rename(output_path, &new_path)?;
        }
    }
    Ok(())
}
