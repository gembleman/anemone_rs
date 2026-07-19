use super::*;

#[test]
fn parses_retry_after_delta_seconds() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::RETRY_AFTER, "17".parse().unwrap());
    assert_eq!(parse_retry_after(&headers), Some(Duration::from_secs(17)));
}

#[test]
fn ignores_invalid_retry_after_without_inventing_a_delay() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::RETRY_AFTER, "invalid".parse().unwrap());
    assert_eq!(parse_retry_after(&headers), None);
}

#[test]
fn bounds_retry_after_to_two_minutes() {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::RETRY_AFTER, "9999".parse().unwrap());
    assert_eq!(parse_retry_after(&headers), Some(Duration::from_secs(120)));
}

fn serve_once(response: Vec<u8>) -> String {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = stream.read(&mut request);
        stream.write_all(&response).unwrap();
    });
    format!("http://{address}/")
}

#[tokio::test]
async fn rejects_oversized_content_length_before_reading_the_body() {
    let length = SUCCESS_BODY_LIMIT + 1;
    let response =
        format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n")
            .into_bytes();
    let url = serve_once(response);
    let response = create_client().get(url).send().await.unwrap();

    assert!(matches!(
        send_and_read_body(response).await,
        Err(TranslationError::ResponseTooLarge {
            limit: SUCCESS_BODY_LIMIT
        })
    ));
}

#[tokio::test]
async fn rejects_chunked_error_body_when_actual_bytes_cross_the_limit() {
    let body = vec![b'x'; ERROR_BODY_LIMIT + 1];
    let mut response = format!(
        "HTTP/1.1 500 Internal Server Error\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:X}\r\n",
        body.len()
    )
    .into_bytes();
    response.extend_from_slice(&body);
    response.extend_from_slice(b"\r\n0\r\n\r\n");
    let url = serve_once(response);
    let response = create_client().get(url).send().await.unwrap();

    assert!(matches!(
        send_and_read_body(response).await,
        Err(TranslationError::ResponseTooLarge {
            limit: ERROR_BODY_LIMIT
        })
    ));
}
