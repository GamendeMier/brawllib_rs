use std::fs::File;
use std::fs;
use std::io::{Read, ErrorKind};
use std::path::Path;

use byteorder::{BigEndian, ReadBytesExt};

use failure::Error;
use failure::bail;

pub fn wiird_load_txt(codeset_path: &Path) -> Result<WiiRDBlock, Error> {
    match fs::read_to_string(codeset_path) {
        Ok(text) => {
            let mut data = vec!();
            for line in text.lines() {
                if line.starts_with("*") {
                    let hex_string = line.replace("*", "").replace(" ", "");
                    let hex_chars: Vec<_> = hex_string.chars().collect();

                    // error checking
                    if hex_chars.iter().any(|x| !x.is_digit(16)) {
                        bail!("text codeset {:?} contains a non-hex character in a code", codeset_path);
                    }
                    if hex_chars.len() > 16 {
                        bail!("text codeset {:?} contains a code that has more than 16 digits", codeset_path);
                    }
                    if hex_chars.len() < 16 {
                        bail!("text codeset {:?} contains a code that has less than 16 digits", codeset_path);
                    }

                    // convert hex string to sequence of bytes
                    for i in 0..8 {
                        let first  = hex_chars[i * 2    ].to_digit(16).unwrap() as u8;
                        let second = hex_chars[i * 2 + 1].to_digit(16).unwrap() as u8;
                        data.push((first << 4) | second);
                    }
                }
            }

            Ok(wiird_codes(&data))
        }
        Err(err) => {
            match err.kind() {
                ErrorKind::InvalidData => {
                    bail!("Failed to read WiiRD codeset {:?}: Please reencode the file as utf8.", codeset_path);
                }
                _ => bail!("Cannot read WiiRD codeset {:?}: {:?}", codeset_path, err),
            }
        }
    }
}

pub fn wiird_load_gct(codeset_path: &Path) -> Result<WiiRDBlock, Error> {
    let mut data: Vec<u8> = vec!();
    match File::open(&codeset_path) {
        Ok(mut file) => {
            if let Err(err) = file.read_to_end(&mut data) {
                bail!("Cannot read WiiRD codeset {:?}: {}", codeset_path, err);
            }
        }
        Err(err) => bail!("Cannot read WiiRD codeset {:?}: {}", codeset_path, err)
    }

    if data.len() < 8 {
        bail!("Not a WiiRD gct codeset file: File size is less than 8 bytes");
    }

    Ok(wiird_codes(&data[8..])) // Skip the header
}

pub fn wiird_codes(data: &[u8]) -> WiiRDBlock {
    // TODO: Extend the length of data to avoid panics due to out of bounds accesses.

    if let ProcessedBlock::Finished(block) = process_block(data, false) {
        block
    } else {
        error!("A block in the script did not terminate, or a termination occured without a block.");
        WiiRDBlock { codes: vec!() }
    }
}

fn process_block(data: &[u8], is_nested: bool) -> ProcessedBlock {
    let mut codes = vec!();
    let mut offset = 0;
    while offset < data.len() {
        // Not every code type uses this, but its safe to just create these for if we need them.
        let use_base_address = data[offset] & 0b00010000 == 0;
        let address = (&data[offset ..]).read_u32::<BigEndian>().unwrap() & 0x1FFFFFF;

        let code = data[offset] & 0b11101110;
        match code {
            0x00 => {
                let value = data[offset + 7];
                let length = (&data[offset + 4..]).read_u16::<BigEndian>().unwrap() as u32 + 1;
                codes.push(WiiRDCode::WriteAndFill8 { use_base_address, address, value, length });
                offset += 8;
            }
            0x02 => {
                let value = (&data[offset + 6..]).read_u16::<BigEndian>().unwrap();
                let length = (&data[offset + 4..]).read_u16::<BigEndian>().unwrap() as u32 + 1;
                codes.push(WiiRDCode::WriteAndFill16 { use_base_address, address, value, length });
                offset += 8;
            }
            0x04 => {
                let value = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap();
                codes.push(WiiRDCode::WriteAndFill32 { use_base_address, address, value });
                offset += 8;
            }
            0x06 => {
                let mut values = vec!();
                let count = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap() as usize;
                for i in 0..count {
                    values.push(data[offset + 8 + i]);
                }
                codes.push(WiiRDCode::StringWrite { use_base_address, address, values });

                offset += 8 + count;

                // align the offset to 8 bytes
                let count_mod = count % 8;
                if count_mod != 0 {
                    offset += 8 - count_mod;
                }
            }
            0x08 => {
                let initial_value = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap();
                let value_size = data[offset + 8];
                let count = ((&data[offset + 8..]).read_u16::<BigEndian>().unwrap() & 0x0FFF) + 1;
                let address_increment = (&data[offset + 10..]).read_u16::<BigEndian>().unwrap();
                let value_increment = (&data[offset + 12..]).read_u32::<BigEndian>().unwrap();
                codes.push(WiiRDCode::SerialWrite { use_base_address, address, initial_value, value_size, count, address_increment, value_increment });
                offset += 16;
            }
            0x20 | 0x22 | 0x24 | 0x26 | 0x28 | 0x2A | 0x2C | 0x2E => {
                let value = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap();
                let lhs_mask = (&data[offset + 4..]).read_u16::<BigEndian>().unwrap();
                let rhs_value = (&data[offset + 6..]).read_u16::<BigEndian>().unwrap();

                let insert_endif = address & 1 != 0;
                let address = address & 0xFFFFFFFE;

                if insert_endif {
                    // TODO: Handle this case, it will be very tricky, will need to do something like
                    // return ProcessedBlock::EndIfIf { .. }
                    //instead of codes.push(WiiRDCode::IfStatement { .. }
                }

                let test = match code {
                    0x20 => IfTest::IsEqual { use_base_address, address, value },
                    0x22 => IfTest::IsNotEqual { use_base_address, address, value },
                    0x24 => IfTest::IsGreaterThan { use_base_address, address, value },
                    0x26 => IfTest::IsLessThan { use_base_address, address, value },
                    0x28 => IfTest::IsEqualMask { use_base_address, address, lhs_mask, rhs_value },
                    0x2A => IfTest::IsNotEqualMask { use_base_address, address, lhs_mask, rhs_value },
                    0x2C => IfTest::IsGreaterThanMask { use_base_address, address, lhs_mask, rhs_value },
                    0x2E => IfTest::IsLessThanMask { use_base_address, address, lhs_mask, rhs_value },
                    _    => unreachable!(),
                };
                offset += 8;

                match process_block(&data[offset..], true) {
                    ProcessedBlock::EndIf { count, then_branch, bytes_processed, reset_base_address_high, reset_pointer_address_high } => {
                        offset += bytes_processed;
                        let else_branch = None;
                        codes.push(WiiRDCode::IfStatement { test, then_branch, else_branch, reset_base_address_high, reset_pointer_address_high });

                        let count = match count {
                            EndIfCount::Infinite   => EndIfCount::Infinite,
                            EndIfCount::Finite (x) => EndIfCount::Finite (x - 1),
                        };
                        let multi_endif = match count {
                            EndIfCount::Infinite => true,
                            EndIfCount::Finite (ref x) => *x > 0,
                        };

                        if multi_endif && is_nested {
                            let then_branch = WiiRDBlock { codes };
                            return ProcessedBlock::EndIf { count, then_branch, bytes_processed: offset, reset_base_address_high, reset_pointer_address_high };
                        }
                        else {
                            codes.push(WiiRDCode::ResetAddressHigh { reset_base_address_high, reset_pointer_address_high });
                        }
                    }
                    _ => {
                        // Need to terminate as we have no idea how many bytes were meant to be processed
                        error!("IfStatement {} did not terminate", code);
                        return ProcessedBlock::Finished (WiiRDBlock { codes: vec!() })
                    }
                };
            }
            0x40 => {
                let add_result = data[offset + 1] & 0b00010000 != 0;
                let add_mem_address_bool = data[offset + 1] & 1 != 0;
                let register_bool = data[offset + 2] & 0b00010000 != 0;
                let register = data[offset + 3] & 0xF;
                let mem_address = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap();

                let add_mem_address = match (add_mem_address_bool, use_base_address) {
                    (true, true)  => AddAddress::BaseAddress,
                    (true, false) => AddAddress::PointerAddress,
                    (false, _)    => AddAddress::None,
                };

                let add_mem_address_gecko_register = if register_bool {
                    Some(register)
                } else {
                    None
                };

                codes.push(WiiRDCode::LoadBaseAddress { add_result, add_mem_address, add_mem_address_gecko_register, mem_address });
                offset += 8;
            }
            0x42 => {
                let add_result = data[offset + 1] & 0b00010000 != 0;
                let add_bool = data[offset + 1] & 1 != 0;
                let register_bool = data[offset + 2] & 0b00010000 != 0;
                let register = data[offset + 3] & 0xF;
                let value = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap();

                let add = match (add_bool, use_base_address) {
                    (true, true)  => AddAddress::BaseAddress,
                    (true, false) => AddAddress::PointerAddress,
                    (false, _)    => AddAddress::None,
                };

                let add_gecko_register = if register_bool {
                    Some(register)
                } else {
                    None
                };

                codes.push(WiiRDCode::SetBaseAddress { add_result, add, add_gecko_register, value });
                offset += 8;
            }
            0x44 => {
                let add_mem_address_bool = data[offset + 1] & 1 != 0;
                let register_bool = data[offset + 2] & 0b00010000 != 0;
                let register = data[offset + 3] & 0xF;
                let mem_address = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap();

                let add_mem_address = match (add_mem_address_bool, use_base_address) {
                    (true, true)  => AddAddress::BaseAddress,
                    (true, false) => AddAddress::PointerAddress,
                    (false, _)    => AddAddress::None,
                };

                let add_mem_address_gecko_register = if register_bool {
                    Some(register)
                } else {
                    None
                };
                codes.push(WiiRDCode::StoreBaseAddress { add_mem_address, add_mem_address_gecko_register, mem_address });
                offset += 8;
            }
            0x46 => {
                let address_offset = (&data[offset + 2..]).read_i16::<BigEndian>().unwrap();
                codes.push(WiiRDCode::SetBaseAddressToCodeLocation { address_offset });
                offset += 8;
            }
            0x48 => {
                let add_result = data[offset + 1] & 0b00010000 != 0;
                let add_mem_address_bool = data[offset + 1] & 1 != 0;
                let register_bool = data[offset + 2] & 0b00010000 != 0;
                let register = data[offset + 3] & 0xF;
                let mem_address = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap();

                let add_mem_address = match (add_mem_address_bool, use_base_address) {
                    (true, true)  => AddAddress::BaseAddress,
                    (true, false) => AddAddress::PointerAddress,
                    (false, _)    => AddAddress::None,
                };

                let add_mem_address_gecko_register = if register_bool {
                    Some(register)
                } else {
                    None
                };

                codes.push(WiiRDCode::LoadPointerAddress { add_result, add_mem_address, add_mem_address_gecko_register, mem_address });
                offset += 8;
            }
            0x4A => {
                let add_result = data[offset + 1] & 0b00010000 != 0;
                let add_bool = data[offset + 1] & 1 != 0;
                let register_bool = data[offset + 2] & 0b00010000 != 0;
                let register = data[offset + 3] & 0xF;
                let value = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap();

                let add = match (add_bool, use_base_address) {
                    (true, true)  => AddAddress::BaseAddress,
                    (true, false) => AddAddress::PointerAddress,
                    (false, _)    => AddAddress::None,
                };

                let add_gecko_register = if register_bool {
                    Some(register)
                } else {
                    None
                };

                codes.push(WiiRDCode::SetPointerAddress { add_result, add, add_gecko_register, value });
                offset += 8;
            }
            0x4C => {
                let add_mem_address_bool = data[offset + 1] & 1 != 0;
                let register_bool = data[offset + 2] & 0b00010000 != 0;
                let register = data[offset + 3] & 0xF;
                let mem_address = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap();

                let add_mem_address = match (add_mem_address_bool, use_base_address) {
                    (true, true)  => AddAddress::BaseAddress,
                    (true, false) => AddAddress::PointerAddress,
                    (false, _)    => AddAddress::None,
                };

                let add_mem_address_gecko_register = if register_bool {
                    Some(register)
                } else {
                    None
                };
                codes.push(WiiRDCode::StorePointerAddress { add_mem_address, add_mem_address_gecko_register, mem_address });
                offset += 8;
            }
            0x4E => {
                let address_offset = (&data[offset + 2..]).read_i16::<BigEndian>().unwrap();
                codes.push(WiiRDCode::SetPointerAddressToCodeLocation { address_offset });
                offset += 8;
            }
            0x60 => {
                let count = (&data[offset + 2..]).read_u16::<BigEndian>().unwrap();
                let block_id = data[offset + 7];
                codes.push(WiiRDCode::SetRepeat { count, block_id });
                offset += 8;
            }
            0x62 => {
                let block_id = data[offset + 7] & 0xF;
                codes.push(WiiRDCode::ExecuteRepeat { block_id });
                offset += 8;
            }
            0x64 => {
                let flag = match data[offset + 1] {
                    0x00 => JumpFlag::WhenTrue,
                    0x10 => JumpFlag::WhenFalse,
                    0x20 => JumpFlag::Always,
                    flag => {
                        error!("Unknown jump flag '{}' in return", flag);
                        return ProcessedBlock::Finished (WiiRDBlock { codes });
                    }
                };
                let block_id = data[offset + 7] & 0xF;
                codes.push(WiiRDCode::Return { flag, block_id });
                offset += 8;
            }
            0x66 => {
                let flag = match data[offset + 1] {
                    0x00 => JumpFlag::WhenTrue,
                    0x10 => JumpFlag::WhenFalse,
                    0x20 => JumpFlag::Always,
                    flag => {
                        error!("Unknown jump flag '{}' in goto", flag);
                        return ProcessedBlock::Finished (WiiRDBlock { codes });
                    }
                };

                let offset_lines = (&data[offset + 2..]).read_i16::<BigEndian>().unwrap();

                offset += 8;

                // Skip over these instructions as its probably a data section.
                // TODO: If I want to use the AST for converting to/from a higher level language
                // this will need to properly handle data sections
                // TODO: Doesnt handle a goto going backwards into the skipped lines later on.
                if let JumpFlag::Always = flag {
                    offset += 8 * offset_lines as usize;
                }
                else {
                    codes.push(WiiRDCode::Goto { flag, offset_lines });
                }
            }
            0x68 => {
                let flag = match data[offset + 1] {
                    0x00 => JumpFlag::WhenTrue,
                    0x10 => JumpFlag::WhenFalse,
                    0x20 => JumpFlag::Always,
                    flag => {
                        error!("Unknown jump flag '{}' in subroutine", flag);
                        return ProcessedBlock::Finished (WiiRDBlock { codes });
                    }
                };
                let offset_lines = (&data[offset + 2..]).read_i16::<BigEndian>().unwrap();
                let block_id = data[offset + 7] & 0xF;
                codes.push(WiiRDCode::Subroutine { flag, offset_lines, block_id });
                offset += 8;
            }
            0x80 => {
                let add_result = data[offset + 1] & 0b00010000 != 0;
                let add_bool = data[offset + 1] & 1 != 0;
                let register = data[offset + 3] & 0xF;
                let value = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap();

                let add = match (add_bool, use_base_address) {
                    (true, true)  => AddAddress::BaseAddress,
                    (true, false) => AddAddress::PointerAddress,
                    (false, _)    => AddAddress::None,
                };

                codes.push(WiiRDCode::SetGeckoRegister { add_result, add, register, value });
                offset += 8;
            }
            0x82 => {
                let register = data[offset + 3] & 0xF;
                let mem_address = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap();
                codes.push(WiiRDCode::LoadGeckoRegister { register, mem_address });
                offset += 8;
            }
            0x84 => {
                let register = data[offset + 3] & 0xF;
                let mem_address = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap();
                codes.push(WiiRDCode::StoreGeckoRegister { register, mem_address });
                offset += 8;
            }
            0x86 => {
                let operation_byte = data[offset + 1] & 0xF0;
                let load_register  = data[offset + 1] & 0b00000001 != 0;
                let load_value     = data[offset + 1] & 0b00000010 != 0;
                let register       = data[offset + 3] & 0x0F;
                let value          = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap();

                let operation = GeckoOperation::new(operation_byte);

                codes.push(WiiRDCode::OperationGeckoRegisterDirectValue { operation, load_register, load_value, register, value });
                offset += 8;
            }
            0x88 => {
                let operation_byte = data[offset + 1] & 0xF0;
                let load_register1 = data[offset + 1] & 0b00000001 != 0;
                let load_register2 = data[offset + 1] & 0b00000010 != 0;
                let register1      = data[offset + 3] & 0x0F;
                let register2      = data[offset + 7] & 0x0F;

                let operation = GeckoOperation::new(operation_byte);

                codes.push(WiiRDCode::OperationGeckoRegister { operation, load_register1, load_register2, register1, register2 });
                offset += 8;
            }
            0x8A => {
                let count           = (&data[offset + 1..]).read_u16::<BigEndian>().unwrap();
                let source_register =   data[offset + 3] & 0xF0;
                let dest_register   =   data[offset + 3] & 0x0F;
                let dest_offset     = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap();

                let dest_register = if dest_register == 0x0F {
                    None
                } else {
                    Some(dest_register)
                };

                codes.push(WiiRDCode::MemoryCopy1 { use_base_address, count, source_register, dest_register, dest_offset });
                offset += 8;
            }
            0x8C => {
                let count           = (&data[offset + 1..]).read_u16::<BigEndian>().unwrap();
                let source_register =   data[offset + 3] & 0xF0;
                let dest_register   =   data[offset + 3] & 0x0F;
                let source_offset   = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap();

                let source_register = if source_register == 0x0F {
                    None
                } else {
                    Some(source_register)
                };

                codes.push(WiiRDCode::MemoryCopy2 { use_base_address, count, source_register, dest_register, source_offset });
                offset += 8;
            }
            0xC0 => {
                let mut instruction_data = vec!();
                let count = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap() as usize;
                for i in 0..count * 8 {
                    instruction_data.push(data[offset + 8 + i]);
                }
                codes.push(WiiRDCode::ExecutePPC { instruction_data });

                offset += 8 + count * 8;
            }
            0xC2 => {
                let mut instruction_data = vec!();
                let count = (&data[offset + 4..]).read_u32::<BigEndian>().unwrap() as usize;
                for i in 0..count * 8 {
                    instruction_data.push(data[offset + 8 + i]);
                }
                codes.push(WiiRDCode::InsertPPC { use_base_address, address, instruction_data });

                offset += 8 + count * 8;
            }
            0xE0 => {
                let reset_base_address_high = (&data[offset + 4..]).read_u16::<BigEndian>().unwrap();
                let reset_pointer_address_high = (&data[offset + 6..]).read_u16::<BigEndian>().unwrap();

                offset += 8;

                if is_nested {
                    return ProcessedBlock::EndIf { count: EndIfCount::Infinite, then_branch: WiiRDBlock { codes }, bytes_processed: offset, reset_base_address_high, reset_pointer_address_high };
                }
                else {
                    codes.push(WiiRDCode::ResetAddressHigh { reset_base_address_high, reset_pointer_address_high });
                }
            }
            0xE2 => {
                let else_branch = data[offset + 1] & 0x10 != 0;
                let count = data[offset + 3];
                let reset_base_address_high = (&data[offset + 4..]).read_u16::<BigEndian>().unwrap();
                let reset_pointer_address_high = (&data[offset + 6..]).read_u16::<BigEndian>().unwrap();

                if else_branch {
                    codes.push(WiiRDCode::Else { endif_count: count, reset_base_address_high, reset_pointer_address_high });
                }

                offset += 8;
                if is_nested {
                    if count != 0 {
                        return ProcessedBlock::EndIf { count: EndIfCount::Finite(count), then_branch: WiiRDBlock { codes }, bytes_processed: offset, reset_base_address_high, reset_pointer_address_high };
                    }
                }
                else {
                    codes.push(WiiRDCode::ResetAddressHigh { reset_base_address_high, reset_pointer_address_high });
                }
            }
            0xF0 => {
                // End of codes
            }
            unknown => {
                // Can't really continue processing because we dont know what the correct offset should be.
                // Report an error and return what we have so far.
                error!("Cannot process WiiRD code starting with 0x{:x}", unknown);
                return ProcessedBlock::Finished (WiiRDBlock { codes });
            }
        }
    }

    ProcessedBlock::Finished(WiiRDBlock { codes })
}

enum ProcessedBlock {
    Finished     (WiiRDBlock),
    EndIf        { count: EndIfCount, then_branch: WiiRDBlock, bytes_processed: usize, reset_base_address_high: u16, reset_pointer_address_high: u16 },
}

#[derive(Clone, Debug)]
pub enum EndIfCount {
    Infinite,
    Finite (u8),
}

#[derive(Serialize, Clone, Debug)]
pub struct WiiRDBlock {
    pub codes: Vec<WiiRDCode>,
}

#[derive(Serialize, Clone, Debug)]
pub enum WiiRDCode {
    /// 00
    WriteAndFill8 { use_base_address: bool, address: u32, value: u8, length: u32 },
    /// 02
    WriteAndFill16 { use_base_address: bool, address: u32, value: u16, length: u32 },
    /// 04
    WriteAndFill32 { use_base_address: bool, address: u32, value: u32 },
    /// 06
    StringWrite { use_base_address: bool, address: u32, values: Vec<u8> },
    /// 08
    SerialWrite { use_base_address: bool, address: u32, initial_value: u32, value_size: u8, count: u16, address_increment: u16, value_increment: u32 },
    /// 20 or 22 or 24 or 26 or 28 or 2A or 2C or 2E
    IfStatement {
        test: IfTest,
        then_branch: WiiRDBlock,
        else_branch: Option<Box<WiiRDBlock>>,
        reset_base_address_high: u16,
        reset_pointer_address_high: u16
    },
    /// 40
    LoadBaseAddress { add_result: bool, add_mem_address: AddAddress, add_mem_address_gecko_register: Option<u8>, mem_address: u32 },
    /// 42
    SetBaseAddress { add_result: bool, add: AddAddress, add_gecko_register: Option<u8>, value: u32 },
    /// 44
    StoreBaseAddress { add_mem_address: AddAddress, add_mem_address_gecko_register: Option<u8>, mem_address: u32 },
    /// 46
    /// Put next code's location into the Base Address
    /// Base address will hold the address at which the next line of code is stored + address_offset
    SetBaseAddressToCodeLocation { address_offset: i16 },
    /// 48
    LoadPointerAddress { add_result: bool, add_mem_address: AddAddress, add_mem_address_gecko_register: Option<u8>, mem_address: u32 },
    /// 48
    SetPointerAddress { add_result: bool, add: AddAddress, add_gecko_register: Option<u8>, value: u32 },
    /// 4C
    StorePointerAddress { add_mem_address: AddAddress, add_mem_address_gecko_register: Option<u8>, mem_address: u32 },
    /// 4E
    /// Put next code's location into the Pointer Address
    /// Pointer will hold the address at which the next line of code is stored + address_offset
    SetPointerAddressToCodeLocation { address_offset: i16 },
    /// 60
    /// Store next code address and count in block_id.
    SetRepeat { count: u16, block_id: u8 },
    /// 62
    ExecuteRepeat { block_id: u8 },
    /// 64
    Return { flag: JumpFlag, block_id: u8 },
    /// 66
    /// The code handler jumps to (next line of code + offset_lines)
    Goto { flag: JumpFlag, offset_lines: i16 },
    /// 68
    /// The code handler stores the next code address in block_id, then it jumps to (next line of code + offset_lines)
    Subroutine { flag: JumpFlag, offset_lines: i16, block_id: u8 },
    /// 80
    SetGeckoRegister { add_result: bool, add: AddAddress, register: u8, value: u32 },
    /// 82
    LoadGeckoRegister { register: u8, mem_address: u32 },
    /// 84
    StoreGeckoRegister { register: u8, mem_address: u32 },
    /// 86
    OperationGeckoRegisterDirectValue { operation: GeckoOperation, load_register: bool, load_value: bool, register: u8, value: u32 },
    /// 88
    OperationGeckoRegister { operation: GeckoOperation, load_register1: bool, load_register2: bool, register1: u8, register2: u8 },
    /// 8A
    MemoryCopy1 { use_base_address: bool, count: u16, source_register: u8, dest_register: Option<u8>, dest_offset: u32 },
    /// 8C
    MemoryCopy2 { use_base_address: bool, count: u16, source_register: Option<u8>, dest_register: u8, source_offset: u32 },
    /// C0
    ExecutePPC { instruction_data: Vec<u8> },
    /// C2
    InsertPPC { use_base_address: bool, address: u32, instruction_data: Vec<u8> },
    /// E0 or E2
    ResetAddressHigh { reset_base_address_high: u16, reset_pointer_address_high: u16 },
    /// E21
    /// Inverts the code execution status.
    /// TODO: Properly explain how code execution works, its like a hacky way to do if statements,
    /// but first I want to fix my if statement implementation to match.
    Else { endif_count: u8, reset_base_address_high: u16, reset_pointer_address_high: u16 },
}

#[derive(Serialize, Clone, Debug)]
pub enum IfTest {
    IsEqual { use_base_address: bool, address: u32, value: u32 },
    IsNotEqual { use_base_address: bool, address: u32, value: u32 },
    IsGreaterThan { use_base_address: bool, address: u32, value: u32 },
    IsLessThan { use_base_address: bool, address: u32, value: u32 },
    IsEqualMask { use_base_address: bool, address: u32, lhs_mask: u16, rhs_value: u16 },
    IsNotEqualMask { use_base_address: bool, address: u32, lhs_mask: u16, rhs_value: u16 },
    IsGreaterThanMask { use_base_address: bool, address: u32, lhs_mask: u16, rhs_value: u16 },
    IsLessThanMask { use_base_address: bool, address: u32, lhs_mask: u16, rhs_value: u16 },
}

#[derive(Serialize, Clone, Debug)]
pub enum JumpFlag {
    WhenTrue,
    WhenFalse,
    Always,
}

#[derive(Serialize, Clone, Debug)]
pub enum AddAddress {
    BaseAddress,
    PointerAddress,
    None
}

#[derive(Serialize, Clone, Debug)]
pub enum GeckoOperation {
    Add,
    Mul,
    Or,
    And,
    Xor,
    ShiftLeft,
    ShiftRight,
    RotateLeft,
    ArithmeticShiftRight,
    FloatAdd,
    FloatMul,
    Unknown (u8),
}

impl GeckoOperation {
    pub(crate) fn new(value: u8) -> Self {
        match value {
            0  => GeckoOperation::Add,
            1  => GeckoOperation::Mul,
            2  => GeckoOperation::Or,
            3  => GeckoOperation::And,
            4  => GeckoOperation::Xor,
            5  => GeckoOperation::ShiftLeft,
            6  => GeckoOperation::ShiftRight,
            7  => GeckoOperation::RotateLeft,
            8  => GeckoOperation::ArithmeticShiftRight,
            10 => GeckoOperation::FloatAdd,
            11 => GeckoOperation::FloatMul,
            _  => GeckoOperation::Unknown (value),
        }
    }
}
