//! Generate clearly-synthetic public test fixtures (never real patches).
//! cargo run -p p6-core --example make_fixtures -- fixtures/public

use p6_core::library::export::{bank_bytes, selection_bytes};
use p6_core::protocol::messages::{edit_buffer_frame, program_file_frame};
use p6_core::protocol::payload::{off, set_name, synthetic_payload};
use p6_core::slot::{StoredAddress, UserSlot};
use p6_core::Payload;

fn styled(seed: u32, name: &str, kind: &str) -> Payload {
    let mut b = *synthetic_payload(seed, name).bytes();
    match kind {
        "pad" => {
            b[off::AMP_ATTACK] = 90;
            b[off::AMP_SUSTAIN] = 110;
            b[off::AMP_RELEASE] = 100;
        }
        "pluck" => {
            b[off::AMP_ATTACK] = 0;
            b[off::AMP_SUSTAIN] = 5;
            b[off::AMP_DECAY] = 40;
            b[off::AMP_RELEASE] = 20;
        }
        "arp" => b[off::ARP_ON] = 1,
        _ => {}
    }
    set_name(&mut b, name);
    Payload::from_slice(&b).unwrap()
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| "fixtures/public".into());
    std::fs::create_dir_all(&dir).unwrap();
    let kinds = ["Bass", "Lead", "Pad", "Keys", "Pluck", "Arp", "FX", "Brass"];
    let bank: Vec<Option<Payload>> = (0..500u32)
        .map(|i| {
            let k = kinds[(i % 8) as usize];
            Some(styled(10_000 + i, &format!("TEST {k} {i:03}"), &k.to_lowercase()))
        })
        .collect();
    std::fs::write(format!("{dir}/synthetic-full-bank.syx"), bank_bytes(&bank).unwrap()).unwrap();

    let sel: Vec<(UserSlot, Payload)> = (0..24u16).map(|i| (UserSlot::new(i).unwrap(), styled(20_000 + i as u32, &format!("TEST Old {} {i:02}", kinds[(i % 8) as usize]), &kinds[(i % 8) as usize].to_lowercase()))).collect();
    let mut mixed = selection_bytes(&sel).unwrap();
    mixed.extend(edit_buffer_frame(&styled(30_000, "TEST Edit Buffer", "pad")));
    mixed.extend([0xF0, 0x42, 0x30, 0x00, 0xF7]); // unrelated manufacturer
    mixed.extend(program_file_frame(StoredAddress::new(7, 12).unwrap(), &styled(30_001, "TEST Factory Addr", "lead")));
    mixed.extend(program_file_frame(StoredAddress::new(0, 3).unwrap(), &sel[3].1)); // repeated address + payload
    std::fs::write(format!("{dir}/synthetic-mixed-archive.syx"), mixed).unwrap();
    println!("wrote fixtures to {dir}");
}
