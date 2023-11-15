use curl::easy::{Easy, List};
use log::debug;

pub struct HttpMetaReader {
    resource_url: String,
    additional_headers: Vec<String>,
}

impl HttpMetaReader {

    pub fn new(url: &str, additional_headers: Vec<String>) -> Self {
        HttpMetaReader {
            resource_url: String::from(url),
            additional_headers,
        }
    }

    pub fn get_file_size(&self) -> usize {
        let mut easy = Easy::new();
        easy.nobody(true).unwrap();
        let mut headers = List::new();
        self.additional_headers.iter().for_each(|x| {
            headers.append(&x).unwrap();
        });
        easy.http_headers(headers).unwrap();
        easy
            .url(&self.resource_url)
            .unwrap();
        easy.perform().unwrap();
        let size = easy.content_length_download().unwrap() as usize;
        debug!("Fetched the size of remote resource: {}", size);
        size
    }
}
