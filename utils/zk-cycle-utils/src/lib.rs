use risc0_zkvm_platform::syscall::SyscallName;

pub fn get_syscall_name() -> SyscallName {
    let cycle_string = "cycle_metrics\0";
    let bytes = cycle_string.as_bytes();
    SyscallName::from_bytes_with_nul(bytes.as_ptr())
}

#[cfg(feature = "native")]
pub fn cycle_count_callback(input: risc0_zkvm::Bytes) -> risc0_zkvm::Result<risc0_zkvm::Bytes> {
    if input.len() == std::mem::size_of::<usize>() {
        let mut array = [0u8; std::mem::size_of::<usize>()];
        array.copy_from_slice(&input);
        println!("== syscall ==> {}", usize::from_le_bytes(array));
    } else {
        println!("NONE");
    }
    Ok(risc0_zkvm::Bytes::new())
}

pub fn get_syscall_name_cycles() -> SyscallName {
    let cycle_string = "cycle_count\0";
    let bytes = cycle_string.as_bytes();
    SyscallName::from_bytes_with_nul(bytes.as_ptr())
}

pub fn print_cycle_count() {
    let metrics_syscall_name = get_syscall_name_cycles();
    let serialized = (risc0_zkvm::guest::env::get_cycle_count() as u64).to_le_bytes();
    risc0_zkvm::guest::env::send_recv_slice::<u8, u8>(metrics_syscall_name, &serialized);
}
