use aoqi_recovered_core::dense_storage::{encode, set_encoded, DenseBlock};

fn main() {
    for discriminant in (0..=0x39).chain(std::iter::once(u64::MAX)) {
        for payload in [0, 0xfedc_ba98_7654_32a5, u64::MAX] {
            for index in [0, 7, 8] {
                let mut blocks = [DenseBlock::sentinel_filled(); 2];
                set_encoded(&mut blocks, 9, index, encode(discriminant, payload).unwrap());
                print!("{discriminant:016x}\t{payload:016x}\t{index}\t");
                for block in blocks {
                    for word in block.payloads {
                        for byte in word.to_le_bytes() {
                            print!("{byte:02x}");
                        }
                    }
                    for tag in block.tags {
                        print!("{tag:02x}");
                    }
                }
                println!();
            }
        }
    }
}
