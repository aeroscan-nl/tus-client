use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use futures::io::Cursor;
use futures::{AsyncRead, AsyncSeek, AsyncSeekExt};
use std::collections::HashMap;
use std::future::Future;
use std::io::SeekFrom;
use std::task::Poll;
use tus_client;
use tus_client::http::{HttpHandler, HttpMethod, HttpRequest, HttpResponse};
use tus_client::{Error, TusExtension};

struct TestHandler {
    pub upload_progress: usize,
    pub total_upload_size: usize,
    pub status_code: usize,
    pub tus_version: String,
    pub extensions: String,
    pub max_upload_size: usize,
}

impl Default for TestHandler {
    fn default() -> Self {
        TestHandler {
            upload_progress: 1234,
            total_upload_size: 2345,
            status_code: 200,
            tus_version: String::from("1.0.0"),
            extensions: String::from(""),
            max_upload_size: 12345,
        }
    }
}

fn unwrap_future<F>(fut: F) -> F::Output
where
    F: Future,
{
    let waker = futures::task::noop_waker();
    let mut context = std::task::Context::from_waker(&waker);
    let mut pin = Box::pin(fut);
    let Poll::Ready(result) = Future::poll(pin.as_mut(), &mut context) else {
        panic!("Future did not resolve");
    };

    result
}

impl HttpHandler for TestHandler {
    async fn handle_request<'a>(&self, req: HttpRequest<'a>) -> Result<HttpResponse, Error> {
        match &req.method {
            HttpMethod::Head => {
                let mut headers = HashMap::new();
                headers.insert(
                    "upload-length".to_owned(),
                    self.total_upload_size.to_string(),
                );
                headers.insert("upload-offset".to_owned(), self.upload_progress.to_string());
                headers.insert(
                    "upload-metadata".to_owned(),
                    STANDARD.encode("key_one:value_one;key_two:value_two;k"),
                );

                Ok(HttpResponse {
                    status_code: self.status_code,
                    headers,
                })
            }
            HttpMethod::Options => {
                let mut headers = HashMap::new();
                headers.insert("tus-version".to_owned(), self.tus_version.clone());
                headers.insert("tus-extension".to_owned(), self.extensions.clone());
                headers.insert("tus-max-size".to_owned(), self.max_upload_size.to_string());

                Ok(HttpResponse {
                    status_code: self.status_code,
                    headers,
                })
            }
            HttpMethod::Patch => {
                let mut headers = HashMap::new();
                headers.insert("tus-version".to_owned(), self.tus_version.clone());
                headers.insert(
                    "upload-offset".to_owned(),
                    (req.body.unwrap().len()
                        + req
                            .headers
                            .get("upload-offset")
                            .unwrap()
                            .parse::<usize>()
                            .unwrap())
                    .to_string(),
                );

                Ok(HttpResponse {
                    status_code: self.status_code,
                    headers,
                })
            }
            HttpMethod::Post => {
                let mut headers = HashMap::new();
                headers.insert("tus-version".to_owned(), self.tus_version.clone());
                headers.insert("location".to_owned(), "/something_else".to_owned());

                Ok(HttpResponse {
                    status_code: self.status_code,
                    headers,
                })
            }
            HttpMethod::Delete => {
                let mut headers = HashMap::new();
                headers.insert("tus-version".to_owned(), self.tus_version.clone());

                Ok(HttpResponse {
                    status_code: self.status_code,
                    headers,
                })
            }
        }
    }
}

fn create_temp_file() -> impl AsyncRead + AsyncSeek + Unpin {
    let buffer: Vec<u8> = (0..(1024 * 763)).map(|_| rand::random::<u8>()).collect();
    Cursor::new(buffer)
}

#[test]
fn should_report_correct_upload_progress() {
    let client = tus_client::Client::new(TestHandler {
        status_code: 204,
        ..TestHandler::default()
    });

    let info = unwrap_future(client.get_info("/something")).expect("'get_progress' call failed");

    let metadata = info.metadata.unwrap();
    assert_eq!(1234, info.bytes_uploaded);
    assert_eq!(2345, info.total_size.unwrap());
    assert_eq!(
        String::from("value_one"),
        metadata.get("key_one").unwrap().to_owned()
    );
    assert_eq!(
        String::from("value_two"),
        metadata.get("key_two").unwrap().to_owned()
    );
}

#[test]
fn should_return_not_found_at_4xx_status() {
    let client = tus_client::Client::new(TestHandler {
        status_code: 400,
        ..TestHandler::default()
    });

    let result = unwrap_future(client.get_info("/something"));

    assert!(result.is_err());
    match result {
        Err(tus_client::Error::NotFoundError) => {}
        _ => panic!("Expected 'Error::NotFoundError'"),
    }
}

#[test]
fn should_return_server_info() {
    let client = tus_client::Client::new(TestHandler {
        status_code: 204,
        tus_version: String::from("1.0.0,0.2.2"),
        extensions: String::from("creation, termination"),
        ..TestHandler::default()
    });

    let result =
        unwrap_future(client.get_server_info("/something")).expect("'get_server_info' call failed");

    assert_eq!(vec!["1.0.0", "0.2.2"], result.supported_versions);
    assert_eq!(
        vec![TusExtension::Creation, TusExtension::Termination],
        result.extensions
    );
    assert_eq!(12345, result.max_upload_size.unwrap());
}

#[test]
fn should_upload_file() {
    let mut temp_file = create_temp_file();

    let client = tus_client::Client::new(TestHandler {
        upload_progress: 0,
        total_upload_size: unwrap_future(temp_file.seek(SeekFrom::End(0))).unwrap() as usize,
        status_code: 204,
        ..TestHandler::default()
    });

    unwrap_future(client.upload("/something", temp_file)).expect("'upload' call failed");
}

#[test]
fn should_upload_file_with_custom_chunk_size() {
    let mut temp_file = create_temp_file();

    let client = tus_client::Client::new(TestHandler {
        upload_progress: 0,
        total_upload_size: unwrap_future(temp_file.seek(SeekFrom::End(0))).unwrap() as usize,
        status_code: 204,
        ..TestHandler::default()
    });

    unwrap_future(client.upload_with_chunk_size("/something", temp_file, 9 * 87 * 65 * 43))
        .expect("'upload_with_chunk_size' call failed");
}

#[test]
fn should_receive_upload_path() {
    let mut temp_file = create_temp_file();

    let client = tus_client::Client::new(TestHandler {
        status_code: 201,
        ..TestHandler::default()
    });

    let mut metadata = HashMap::new();
    metadata.insert("key_one".to_owned(), "value_one".to_owned());
    metadata.insert("key_two".to_owned(), "value_two".to_owned());

    let result = unwrap_future(client.create(
        "/something",
        unwrap_future(temp_file.seek(SeekFrom::End(0))).unwrap() as usize,
    ))
    .expect("'create_with_metadata' call failed");

    assert!(!result.is_empty());
}

#[test]
fn should_receive_upload_path_with_metadata() {
    let mut temp_file = create_temp_file();

    let client = tus_client::Client::new(TestHandler {
        status_code: 201,
        ..TestHandler::default()
    });

    let mut metadata = HashMap::new();
    metadata.insert("key_one".to_owned(), "value_one".to_owned());
    metadata.insert("key_two".to_owned(), "value_two".to_owned());

    let result = unwrap_future(client.create_with_metadata(
        "/something",
        unwrap_future(temp_file.seek(SeekFrom::End(0))).unwrap() as usize,
        metadata,
    ))
    .expect("'create_with_metadata' call failed");

    assert!(!result.is_empty());
}

#[test]
fn should_receive_204_after_deleting_file() {
    let client = tus_client::Client::new(TestHandler {
        status_code: 204,
        ..TestHandler::default()
    });

    unwrap_future(client.delete("/something")).expect("'delete' call failed");
}
