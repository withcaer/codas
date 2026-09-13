#![cfg(test)]
//! Integration tests of the codec over `std` IO sockets.
extern crate alloc;

use std::{
    net::{TcpListener, TcpStream},
    thread::JoinHandle,
};

use codas::{
    codec::{CodecError, ReadsDecodable, WritesEncodable},
    types::Text,
};

#[test]
pub fn test_codec_over_tcp() -> Result<(), CodecError> {
    // Create our request data.
    let request_data = TestMessage {
        number: 9000,
        text_list: vec!["I like cake.".into()],
        text: "Hello, Codecs!".into(),
    };

    // We'll add this string to the response,
    // and increment the response number by 1.
    let additional_string = Text::from("The cake is a lie.");

    // Create our _expected_ response data.
    let expected_response_data = TestMessage {
        number: 9001,
        text_list: vec!["I like cake.".into(), additional_string.clone()],
        text: "Hello, Codecs!".into(),
    };

    // Create TCP listener on an arbitrary port, configuring
    // it to echo our request with some additional data.
    let listener = TcpListener::bind("0.0.0.0:0").unwrap();
    let listener_port = listener.local_addr().unwrap().port();
    let expected_request_data = request_data.clone();
    let server: JoinHandle<Result<(), CodecError>> = std::thread::spawn(move || {
        // Accept the first client request and decode it.
        let (mut socket, _) = listener.accept().unwrap();
        let mut request_data = socket.read_data()?;
        assert_eq!(expected_request_data, request_data);

        // Append additional data to the test data.
        request_data.number += 1;
        request_data.text_list.push(additional_string);

        // Send it back to the client.
        socket.write_data(&request_data)?;

        Ok(())
    });

    // Create a TCP client connection to the listener
    // and send some encoded data.
    let mut client = TcpStream::connect(format!("127.0.0.1:{listener_port}")).unwrap();
    client.write_data(&request_data)?;

    // Decode the response.
    let response_data = client.read_data()?;
    assert_eq!(expected_response_data, response_data);

    // Join server to ensure no errors occurred on it's side.
    server.join().unwrap()?;

    Ok(())
}

/// Data types unknown to a coda decode as `Unspecified`.
#[test]
pub fn unknown_data_types_decode_as_unspecified() -> Result<(), CodecError> {
    use codas::codec::{DataFormat, DataHeader};

    // Encode data with an ordinal the `Test` coda doesn't define.
    let mut bytes = vec![];
    bytes.write_data(&DataHeader {
        count: 1,
        format: DataFormat {
            blob_size: 4,
            data_fields: 0,
            ordinal: 2,
        },
    })?;
    bytes.write_data(&7u32)?;

    let decoded: TestData = bytes.as_slice().read_data()?;
    assert!(matches!(
        decoded,
        TestData::Unspecified(codas::types::Unspecified::Data { .. })
    ));

    let mut re_encoded = vec![];
    re_encoded.write_data(&decoded)?;
    assert_eq!(bytes, re_encoded);

    Ok(())
}

codas_macros::export_coda!("codas/tests/test_coda.md");
