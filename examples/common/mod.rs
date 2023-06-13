use bytecheck::CheckBytes;
use rkyv::{Archive, Deserialize, Serialize};

/// Example data-structure shared between writer and reader(s)
#[derive(Archive, Deserialize, Serialize, Debug, PartialEq)]
#[archive_attr(derive(CheckBytes))]
pub struct HelloWorld {
    pub version: u32,
    pub messages: Vec<String>,
}
