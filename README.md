## httpfs

This small utility allows you to work with HTTP resource like local file.


## Installation

Ensure `rustup` and `cargo`

```bash
cargo build --release
./target/release/httpfs --help
```

You `--help` option to show help:
```bash
httpfs --help
Usage: httpfs [OPTIONS] <MOUNT_POINT> <URL>

Arguments:
<MOUNT_POINT>  Act as a client, and mount FUSE at given path
<URL>          Remote HTTP resource url

Options:
--auto_unmount                           Automatically unmount on process exit
--additional_header <additional_header>  Additional header will be added to HTTP requests
--allow_root                             Allow root user to access filesystem
-h, --help                                   Print help
```


## Presently supported:

- Serial and random access to file
- Optimized work with HTTP resource using internal buffer and several parallel readers
- Split serial and random read and avoid reading unnecessary data and many small requests


## Restrictions
- Now only one file may be mounted via one process (no support "folders")
- Only read requests is possible

## What should be done first
- Add tests coverage
- CI
- Replace loop+sleep to locks
