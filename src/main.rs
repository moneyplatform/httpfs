use clap::{Arg, ArgAction, Command};
use fuser::{MountOption};
use log::debug;

use crate::file_system::HttpFs;
use crate::http_meta_reader::HttpMetaReader;

mod file_system;
mod http_reader;
mod http_meta_reader;

fn main() {
    env_logger::init();

    let matches = Command::new("hello")
        .arg(
            Arg::new("MOUNT_POINT")
                .required(true)
                .index(1)
                .help("Act as a client, and mount FUSE at given path"),
        )
        .arg(
            Arg::new("URL")
                .required(true)
                .index(2)
                .help("Remote HTTP resource url"),
        )
        .arg(
            Arg::new("auto_unmount")
                .long("auto_unmount")
                .action(ArgAction::SetTrue)
                .help("Automatically unmount on process exit"),
        )
        .arg(
            Arg::new("additional_header")
                .long("additional_header")
                .action(ArgAction::Append)
                .help("Additional header will be added to HTTP requests"),
        )
        .arg(
            Arg::new("allow_root")
                .long("allow_root")
                .action(ArgAction::SetTrue)
                .help("Allow root user to access filesystem"),
        )
        .get_matches();

    let mountpoint = matches.get_one::<String>("MOUNT_POINT").unwrap();
    let resource_url = matches.get_one::<String>("URL").unwrap();
    let mut options = vec![
        MountOption::RO,
        MountOption::FSName("httpfs".to_string()),
    ];
    if matches.get_flag("auto_unmount") {
        options.push(MountOption::AutoUnmount);
    }
    if matches.get_flag("allow_root") {
        options.push(MountOption::AllowRoot);
    }
    let additional_headers: Vec<String> = matches.get_many::<String>("additional_header")
        .unwrap()
        .map(|x| x.to_string())
        .collect();

    let meta_reader = HttpMetaReader::new(resource_url, additional_headers.clone());
    let fs = HttpFs::new(resource_url, meta_reader.get_file_size(), "file", additional_headers.clone());

    fuser::mount2(fs, mountpoint, &options).unwrap();

    debug!("End work");
}
