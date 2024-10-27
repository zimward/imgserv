use std::{
    env,
    fs::{read_dir, read_to_string, write},
    path::PathBuf,
};

use minify_html::minify;

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    let cfg = minify_html::Cfg::spec_compliant();
    for file in read_dir("./data").unwrap() {
        let file = file.unwrap();
        if let Ok(ft) = file.file_type() {
            if ft.is_file() {
                let cont = read_to_string(file.path()).unwrap();
                let minified = minify(cont.as_bytes(), &cfg);
                let compressed = zstd::bulk::compress(&minified, 19);
                //ok this is ugly
                let out = // File::create_new(
                out_dir.join(format!(
                    "{}.zstd",
                    file.path().file_name().unwrap().to_str().unwrap()
                ));
                write(out, compressed.unwrap()).unwrap();
            }
        }
    }
}
