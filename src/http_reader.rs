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
    ordinal_number: usize, // just for logging
}

impl HttpReader {
    pub fn new(
        url: &str,
        start_offset: usize,
        resource_size: usize,
        additional_headers: Vec<String>,
        ordinal_number: usize,
    ) -> Self {
        HttpReader {
            data: Arc::new(Mutex::new(vec![])),
            offset: Arc::new(Mutex::new(start_offset)),
            resource_size,
            resource_url: String::from(url),
            should_stop: Arc::new(Mutex::new(false)),
            additional_headers,
            ordinal_number,
        }
    }

    // Returns requested data from internal buffer or None if requested data isn't exists.
    // Does left trim buffer if it required (leaning on MAX_BUFFER_PREPEND).
    pub fn try_drain_data(&self, abs_addr: DataAddr) -> Option<Vec<u8>> {
        debug!("[reader {}] Start draining data", self.ordinal_number);
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
        debug!("[reader {}] Preparing to write block {:?}", self.ordinal_number, rel_addr.offset..end);
        let requested_data = data[rel_addr.offset..end]
            .to_vec()
            .clone();

        debug!("[reader {}] Removing part of data {:?}", self.ordinal_number, 0..rel_addr.offset);
        *data = data[rel_addr.offset..].to_vec().clone();
        *offset += rel_addr.offset;

        debug!("[reader {}] End drain data. Current offset {}, length {}", self.ordinal_number, offset, data.len());
        Some(requested_data)
    }

    // Returns true if you managed to get the necessary data.
    fn wait_for_data(&self, abs_addr: DataAddr) -> bool {
        // Really data downloading may be in progress, because we need to check data availability.
        let end = min(abs_addr.get_data_end_position(), self.resource_size);
        debug!("[reader {}] Waiting to read data block {:?} from http. Current data {:?}",
            self.ordinal_number,[abs_addr.offset..end], [self.get_offset()..self.get_offset() + self.get_data_len()]);
        let mut total_waited = 0;
        while self.get_offset() + self.get_data_len() < end {
            sleep(Duration::from_millis(BUFFER_FILL_RECHECK_MS));
            total_waited += BUFFER_FILL_RECHECK_MS;
            if total_waited > MAX_RESPONSE_AWAIT_MS {
                warn!("[reader {}] The time to wait the data is over!", self.ordinal_number,);
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
            debug!("[reader {}] Requested offset {} less than existing {}",
                self.ordinal_number, abs_addr.offset, reader_offset);
            return None;
        }
        let reader_possibly_data_reach = reader_offset + MAX_BUFFER_SIZE;
        if abs_addr.get_data_end_position() > reader_possibly_data_reach {
            debug!("[reader {}] Requested data {:?} can not be reached for reader {:?}",
                self.ordinal_number,
                [abs_addr.offset..abs_addr.get_data_end_position()],
                [reader_offset..reader_possibly_data_reach]
            );
            return None;
        }
        let local_addr = DataAddr {
            offset: abs_addr.offset - reader_offset,
            size: abs_addr.size,
        };
        debug!("[reader {}] Translated absolute addr {:?} to local {:?}", self.ordinal_number, abs_addr, local_addr);
        Some(local_addr)
    }

    pub fn fetching_loop(&self) {
        debug!("[reader {}] Setup URL fetching", self.ordinal_number);
        let mut easy = Easy::new();
        easy.buffer_size(16384).unwrap();
        easy.url(&self.resource_url).unwrap();

        let mut headers = List::new();
        let header = format!("Range: bytes={}-", self.get_offset());
        headers.append(&header).unwrap();
        self.additional_headers.iter().for_each(|x| {
            headers.append(&x).unwrap();
        });

        debug!("[reader {}] CURL: Using headers {:?}", self.ordinal_number, headers);

        easy.http_headers(headers).unwrap();

        let mut transfer = easy.transfer();
        transfer.write_function(|buf| {
            let mut total_slept = 0;
            let data_len = self.get_data_len();
            while data_len >= MAX_BUFFER_SIZE {
                sleep(Duration::from_millis(BUFFER_FILL_RECHECK_MS));
                if total_slept == 0 {
                    // Writing log only in first iteration
                    debug!("[reader {}] Sleeping because buffer is full. Current data range: {:?}-{:?}",
                        self.ordinal_number, self.get_offset(), self.get_offset()+data_len);
                }
                total_slept += BUFFER_FILL_RECHECK_MS;
                if self.should_stop() {
                    debug!("[reader {}] Stop fetching loop", self.ordinal_number);
                    return Ok(0);
                }
            }
            if total_slept > 0 {
                debug!("[reader {}] Waked up from sleeping {} ms", self.ordinal_number, total_slept);
            }
            let data = Arc::clone(&self.data);
            let mut _data = data.lock().unwrap();
            _data.extend(buf);
            debug!("[reader {}] Updated data buffer by {} bytes, new len {}",
                self.ordinal_number, buf.len(), _data.len());

            Ok(buf.len())
        }).unwrap();

        debug!("[reader {}] Performing URL fetching", self.ordinal_number);
        let res = transfer.perform();
        debug!("[reader {}] Finished performing URL fetching", self.ordinal_number);

        match res {
            Ok(_) => {}
            Err(e) => debug!("[reader {}] Write function returns error:  {}", self.ordinal_number, e)
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
