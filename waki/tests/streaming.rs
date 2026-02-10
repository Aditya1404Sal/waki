use std::io::{Cursor, Read};
use waki::multipart::{StreamingContent, StreamingForm, StreamingFormReader, StreamingPart};

#[test]
fn test_streaming_part_from_reader() {
    let data = b"Hello, streaming world!";
    let cursor = Cursor::new(data);

    let part = StreamingPart::from_reader("test", cursor)
        .filename("test.txt")
        .mime_str("text/plain")
        .expect("Failed to set mime type");

    assert_eq!(part.key, "test");
    assert_eq!(part.filename, Some("test.txt".to_string()));
    assert!(part.mime.is_some());
}

#[test]
fn test_streaming_part_text() {
    let part = StreamingPart::text("key", "value");

    assert_eq!(part.key, "key");
    assert!(part.filename.is_none());
    assert!(matches!(part.content, StreamingContent::Bytes(_)));
}

#[test]
fn test_streaming_form_builder() {
    let form = StreamingForm::new()
        .text("field1", "value1")
        .text("field2", "value2");

    assert!(!form.boundary().is_empty());
    assert!(form.boundary().starts_with("--FormBoundary"));
}

#[test]
fn test_streaming_form_reader_with_text_only() {
    let form = StreamingForm::new().text("name", "John Doe");

    let mut reader = form.into_reader();
    let mut output = String::new();
    reader
        .read_to_string(&mut output)
        .expect("Failed to read from streaming form");

    // Should contain the boundary and field data
    assert!(output.contains("content-disposition: form-data; name=name"));
    assert!(output.contains("John Doe"));
}

#[test]
fn test_streaming_form_reader_with_mixed_content() {
    let data = b"Binary content here";
    let cursor = Cursor::new(data);

    let form = StreamingForm::new().text("text_field", "Some text").part(
        StreamingPart::from_reader("file_field", cursor)
            .filename("data.bin")
            .mime_str("application/octet-stream")
            .expect("Failed to set mime"),
    );

    let mut reader = form.into_reader();
    let mut output = Vec::new();
    reader
        .read_to_end(&mut output)
        .expect("Failed to read from streaming form");

    let output_str = String::from_utf8_lossy(&output);

    // Verify multipart structure
    assert!(output_str.contains("content-disposition: form-data; name=text_field"));
    assert!(output_str.contains("Some text"));
    assert!(output_str.contains("content-disposition: form-data; name=file_field"));
    assert!(output_str.contains("filename=\"data.bin\""));
    assert!(output_str.contains("content-type: application/octet-stream"));
    assert!(output.windows(data.len()).any(|window| window == data));
}

#[test]
fn test_streaming_form_reader_chunks_correctly() {
    // Create a form with substantial data
    let large_data = vec![b'X'; 1024]; // 1KB of X's
    let cursor = Cursor::new(large_data.clone());

    let form = StreamingForm::new().part(
        StreamingPart::from_reader("file", cursor)
            .filename("large.txt")
            .mime_str("text/plain")
            .expect("Failed to set mime"),
    );

    let mut reader = form.into_reader();

    // Read in small chunks to test chunking behavior
    let mut total_read = 0;
    let mut buffer = [0u8; 64]; // Small buffer to force multiple reads

    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break, // EOF
            Ok(n) => {
                total_read += n;
            }
            Err(e) => panic!("Read error: {}", e),
        }
    }

    // Should have read all the data plus headers and boundaries
    assert!(
        total_read > large_data.len(),
        "Should read more than just file content due to headers"
    );
}

#[test]
fn test_streaming_form_reader_eof() {
    let form = StreamingForm::new().text("field", "value");
    let mut reader = form.into_reader();

    let mut buffer = Vec::new();
    reader
        .read_to_end(&mut buffer)
        .expect("Failed to read to end");

    // Further reads should return 0 (EOF)
    let mut extra = [0u8; 10];
    let n = reader.read(&mut extra).expect("Failed to read after EOF");
    assert_eq!(n, 0, "Should return 0 bytes at EOF");
}

#[test]
fn test_streaming_part_reader_content_enum() {
    // Test Bytes variant
    let bytes_part = StreamingPart::text("key", vec![1, 2, 3]);
    assert!(matches!(bytes_part.content, StreamingContent::Bytes(_)));

    // Test Reader variant
    let cursor = Cursor::new(vec![4, 5, 6]);
    let reader_part = StreamingPart::from_reader("key", cursor);
    assert!(matches!(reader_part.content, StreamingContent::Reader(_)));
}

#[test]
fn test_multiple_parts_ordering() {
    let form = StreamingForm::new()
        .text("first", "1")
        .text("second", "2")
        .text("third", "3");

    let mut reader = form.into_reader();
    let mut output = String::new();
    reader.read_to_string(&mut output).expect("Failed to read");

    // Verify parts appear in order
    let first_pos = output.find("name=first").expect("first field not found");
    let second_pos = output.find("name=second").expect("second field not found");
    let third_pos = output.find("name=third").expect("third field not found");

    assert!(first_pos < second_pos, "Fields should appear in order");
    assert!(second_pos < third_pos, "Fields should appear in order");
}

#[test]
fn test_empty_form() {
    let form = StreamingForm::new();
    let mut reader = form.into_reader();
    let mut output = String::new();
    reader.read_to_string(&mut output).expect("Failed to read");

    // Should only contain final boundary
    assert!(output.contains("--FormBoundary"));
}
