use std::cmp::min;
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;

use curl::easy::{Easy, List};
use log::{debug, warn};

const MAX_BUFFER_SIZE: usize = 1024 * 1024;
const MAX_RESPONSE_AWAIT_MS: u64 = 10000;
// How to often check the buffer is filled
const BUFFER_FILL_RECHECK_MS: u64 = 10;

#[derive(PartialEq, Debug, Clone, Copy)]
pub struct DataAddr {
    offset: usize,
    size: usize,
}

impl DataAddr {
    pub fn new(_offset: usize, _size: usize) -> Self {
        Self {
            offset: _offset,
            size: _size,
        }
    }
    fn get_data_end_position(&self) -> usize {
        self.size + self.offset
    }
}

#[derive()]
pub struct HttpReader {
    data: Arc<Mutex<Vec<u8>>>,
    offset: Arc<Mutex<usize>>,
    resource_size: usize,
    resource_url: String,
    should_stop: Arc<Mutex<bool>>,
    additional_headers: Vec<String>,
}

impl HttpReader {
    pub fn new(
        url: &str,
        start_offset: usize,
        resource_size: usize,
        additional_headers: Vec<String>,
    ) -> Self {
        HttpReader {
            data: Arc::new(Mutex::new(vec![])),
            offset: Arc::new(Mutex::new(start_offset)),
            resource_size,
            resource_url: String::from(url),
            should_stop: Arc::new(Mutex::new(false)),
            additional_headers,
        }
    }

    // Returns requested data from internal buffer or None if requested data isn't exists.
    // Does left trim buffer if it required (leaning on MAX_BUFFER_PREPEND).
    pub fn try_drain_data(&self, abs_addr: DataAddr) -> Option<Vec<u8>> {
        debug!("-------> Start drain data");
        let rel_addr = match self.abs_to_rel_addr(abs_addr) {
            None => { return None; }
            Some(data) => { data }
        };

        if !self.wait_for_data(abs_addr) {
            return None;
        }

        let data_arc = Arc::clone(&self.data);
        let mut data = data_arc.lock().unwrap();
        let offset_arc = Arc::clone(&self.offset);
        let mut offset = offset_arc.lock().unwrap();

        let end = min(data.len(), rel_addr.get_data_end_position());
        debug!("-------> Preparing to write block {:?}", rel_addr.offset..end);
        let requested_data = data[rel_addr.offset..end]
            .to_vec()
            .clone();

        debug!("x------- Removing part of data {:?}", 0..rel_addr.offset);
        *data = data[rel_addr.offset..].to_vec().clone();
        *offset += rel_addr.offset;

        debug!("-------> End drain data. Current offset {}, length {}", offset, data.len());
        Some(requested_data)
    }

    // Returns true if you managed to get the necessary data.
    fn wait_for_data(&self, abs_addr: DataAddr) -> bool {
        // Really data downloading may be in progress, because we need to check data availability.
        let end = min(abs_addr.get_data_end_position(), self.resource_size);
        debug!("S------- Waiting to read data block {:?} from http. Current data {:?}",
            [abs_addr.offset..end], [self.get_offset()..self.get_offset() + self.get_data_len()]);
        let mut total_waited = 0;
        while self.get_offset() + self.get_data_len() < end {
            sleep(Duration::from_millis(BUFFER_FILL_RECHECK_MS));
            total_waited += BUFFER_FILL_RECHECK_MS;
            if total_waited > MAX_RESPONSE_AWAIT_MS {
                warn!("The time to wait the data is over!");
                return false;
            }
        }
        return true;
    }

    fn get_offset(&self) -> usize {
        let arc = Arc::clone(&self.offset);
        let _offset = arc.lock().unwrap();
        *_offset
    }

    // Validates requested data position in file and returns position of this data in local buffer.
    // Returns None if requested data not in current buffer.
    fn abs_to_rel_addr(&self, abs_addr: DataAddr) -> Option<DataAddr> {
        let reader_offset = self.get_offset();
        if abs_addr.offset < reader_offset {
            debug!("Requested offset {} less than existing {}", abs_addr.offset, reader_offset);
            return None;
        }
        let reader_possibly_data_reach = reader_offset + MAX_BUFFER_SIZE;
        if abs_addr.get_data_end_position() > reader_possibly_data_reach {
            debug!("Requested data {:?} can not be reached for reader {:?}",
                [abs_addr.offset..abs_addr.get_data_end_position()], [reader_offset..reader_possibly_data_reach]);
            return None;
        }
        let local_addr = DataAddr {
            offset: abs_addr.offset - reader_offset,
            size: abs_addr.size,
        };
        debug!("Translated absolute addr {:?} to local {:?}", abs_addr, local_addr);
        Some(local_addr)
    }

    pub fn fetching_loop(&self) {
        debug!("<------- Setup URL fetching");
        let mut easy = Easy::new();
        easy.buffer_size(16384).unwrap();
        easy.url(&self.resource_url).unwrap();

        let mut headers = List::new();
        let header = format!("Range: bytes={}-", self.get_offset());
        headers.append(&header).unwrap();
        self.additional_headers.iter().for_each(|x| {
            headers.append(&x).unwrap();
        });

        debug!("CURL: Using headers {:?}", headers);

        easy.http_headers(headers).unwrap();

        let mut transfer = easy.transfer();
        transfer.write_function(|buf| {
            let mut total_slept = 0;
            let data_len = self.get_data_len();
            while data_len >= MAX_BUFFER_SIZE {
                sleep(Duration::from_millis(BUFFER_FILL_RECHECK_MS));
                if total_slept == 0 {
                    // Writing log only in first iteration
                    debug!("<------- Sleeping because buffer is full. Current data range: {:?}-{:?}",
                        self.get_offset(), data_len);
                }
                total_slept += BUFFER_FILL_RECHECK_MS;
                if self.should_stop() {
                    debug!("Stop fetching loop");
                    return Ok(0);
                }
            }
            if total_slept > 0 {
                debug!("<------- Waked up from sleeping {} ms", total_slept);
            }
            let data = Arc::clone(&self.data);
            let mut _data = data.lock().unwrap();
            _data.extend(buf);
            debug!("<------- Updated data buffer by {} bytes, new len {}", buf.len(), _data.len());

            Ok(buf.len())
        }).unwrap();

        debug!("<------- Performing URL fetching");
        let res = transfer.perform();
        debug!("<------- Finished performing URL fetching");

        match res {
            Ok(_) => {}
            Err(e) => debug!("<------- Write function returns error:  {}", e)
        }
    }

    fn get_data_len(&self) -> usize {
        let arc = Arc::clone(&self.data);
        let data = arc.lock().unwrap();
        data.len()
    }

    fn should_stop(&self) -> bool {
        let arc = Arc::clone(&self.should_stop);
        let should_stop = arc.lock().unwrap();
        *should_stop
    }

    pub fn stop(&self) {
        let arc = Arc::clone(&self.should_stop);
        let mut should_stop = arc.lock().unwrap();
        *should_stop = true
    }
}
