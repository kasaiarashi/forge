pub mod backend;
pub mod chunk_store;
pub mod object_store;

// Forward-looking alias. Callers targeting the trait-driven design
// should reach for `FsObjectStore`; the `ChunkStore` name stays for
// backward compatibility.
pub use chunk_store::ChunkStore as FsObjectStore;
