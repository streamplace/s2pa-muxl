use std::env;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::process;

fn usage() -> ! {
    eprintln!("Usage: muxl <command> [args...]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  canonicalize <input.mp4> <output.mp4>   Canonicalize an MP4 file");
    eprintln!("  fragment <input.mp4> <output_dir>       Fragment into per-frame CMAF");
    process::exit(1);
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        usage();
    }

    let result = match args[1].as_str() {
        "canonicalize" => cmd_canonicalize(&args[2..]),
        "fragment" => cmd_fragment(&args[2..]),
        _ => {
            eprintln!("Unknown command: {}", args[1]);
            usage();
        }
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        process::exit(1);
    }
}

fn cmd_canonicalize(args: &[String]) -> muxl::Result<()> {
    if args.len() != 2 {
        eprintln!("Usage: muxl canonicalize <input.mp4> <output.mp4>");
        process::exit(1);
    }
    let input = BufReader::new(File::open(&args[0])?);
    let output = File::create(&args[1])?;
    muxl::canonicalize(input, output)
}

fn cmd_fragment(args: &[String]) -> muxl::Result<()> {
    if args.len() != 2 {
        eprintln!("Usage: muxl fragment <input.mp4> <output_dir>");
        process::exit(1);
    }
    let input = BufReader::new(File::open(&args[0])?);
    let output_dir = Path::new(&args[1]);
    let stats = muxl::fragment_to_directory(input, output_dir)?;

    for track in &stats.tracks {
        eprintln!(
            "track {}: {} ({}) — {} samples, {} bytes",
            track.track_id,
            track.handler_type,
            track.timescale,
            track.sample_count,
            track.total_bytes
        );
    }

    Ok(())
}
