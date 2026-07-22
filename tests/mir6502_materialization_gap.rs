use std::path::Path;

use actionc::codegen::CODE_ORIGIN;
use actionc::includes::load_program_with_expanded_source;
use actionc::mir6502;
use actionc::nir;
use actionc::semantic::{analyze, ir};

#[test]
fn cast_store_consumers_materialize_without_virtual_temps_or_cast_pseudos() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("casts.act");

    assert!(!formatted.contains(" v"));
    assert!(!formatted.contains("extend"));
    assert!(!formatted.contains("truncate"));
    assert!(formatted.contains("store.b global g1+0, a"));
    assert!(formatted.contains("store.b global g1+1, a"));
    assert!(formatted.contains("store.b global g0+0, a"));
    assert!(!bytes.is_empty());
}

#[test]
fn local_scalar_symbol_initializer_aliases_global_storage() {
    let source = "BYTE state PROC Main() BYTE high=state+1 high=$42 RETURN";
    let tokens = actionc::lexer::tokenize(source).expect("tokenize source");
    let program = actionc::parser::parse(&tokens).expect("parse source");
    let model = analyze(&program).expect("analyze source");
    let semir = ir::lower_program(&program, &model);
    let nir = nir::optimize_program(&nir::lower_program(&semir)).expect("optimize NIR");
    let mir = mir6502::lower_program(&nir).expect("lower MIR6502");
    let formatted = mir6502::format_program(&mir);

    assert!(formatted.contains("store.b global g0+1"), "{formatted}");
    assert!(!formatted.contains("local l0"), "{formatted}");
}

#[test]
fn byte_return_values_materialize_through_public_return_slot() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("func_returns_byte.act");

    assert!(formatted.contains("store.b fixed_zp $A0, a"));
    assert!(formatted.contains("a =.b load fixed_zp $A0"));
    assert!(!formatted.contains("result=v"));
    assert!(!formatted.contains("spill sp"));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x85, 0xA0]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0xA5, 0xA0]));
}

#[test]
fn word_return_values_materialize_low_and_high_return_slots() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("func_returns_word.act");

    assert!(formatted.contains("store.b fixed_zp $A0, a"));
    assert!(formatted.contains("store.b fixed_zp $A1, a"));
    assert!(formatted.contains("a =.b load fixed_zp $A0"));
    assert!(formatted.contains("a =.b load fixed_zp $A1"));
    assert!(!formatted.contains("result=v"));
    assert!(!formatted.contains("spill sp"));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x85, 0xA0]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x85, 0xA1]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0xA5, 0xA0]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0xA5, 0xA1]));
}

#[test]
fn word_return_zero_clears_low_and_high_return_slots() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("func_returns_word_zero.act");

    assert!(formatted.contains("store.b fixed_zp $A0, a"));
    assert!(formatted.contains("store.b fixed_zp $A1, a"));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x85, 0xA0]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x85, 0xA1]));
}

#[test]
fn direct_byte_call_argument_materializes_to_register_home() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("builtin_putchar_byte.act");

    assert!(formatted.contains("call Put@$A4CE args=[a.b -> a]"));
    assert!(!formatted.contains("args=[v"));
    assert!(bytes.windows(3).any(|bytes| bytes == [0x4C, 0xCE, 0xA4]));
    assert!(!bytes.windows(3).any(|bytes| bytes == [0x20, 0xCE, 0xA4]));
}

#[test]
fn direct_word_address_call_argument_materializes_to_register_pair_home() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("call_with_address_arg.act");

    assert!(formatted.contains("call r0 args=[a.b -> a, x.b -> x]"));
    assert!(!formatted.contains("args=[v"));
    assert!(bytes.windows(3).any(|bytes| bytes[0] == 0x4C));
    assert!(!bytes.windows(3).any(|bytes| bytes[0] == 0x20));
}

#[test]
fn pointer_cell_const_read_materializes_through_fixed_zp_pair() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("unsized_byte_array_const_read.act");

    assert!(formatted.contains("store.b fixed_zp $AC, a"));
    assert!(formatted.contains("store.b fixed_zp $AD, a"));
    assert!(formatted.contains("load_indirect (zp$AC),y+2"));
    assert!(!formatted.contains("pointer-cell addresses"));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x85, 0xAC]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x85, 0xAD]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0xB1, 0xAC]));
}

#[test]
fn pointer_deref_read_consumers_store_directly_without_spills() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("word_pointer_deref_read.act");

    assert!(formatted.contains("a =.b load_indirect (zp$AC),y+0"));
    assert!(formatted.contains("store.b global g1+0, a"));
    assert!(formatted.contains("a =.b load_indirect (zp$AC),y+1"));
    assert!(formatted.contains("store.b global g1+1, a"));
    assert!(!formatted.contains("spill sp"));
    assert!(bytes.windows(2).any(|bytes| bytes == [0xB1, 0xAC]));
    assert!(bytes.windows(3).any(|bytes| bytes == [0x8D, 0x02, 0x30]));
    assert!(bytes.windows(3).any(|bytes| bytes == [0x8D, 0x03, 0x30]));
}

#[test]
fn repeated_pointer_store_read_reuses_staged_address_pair() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("pointer_store_read_reuse.act");

    assert_eq!(formatted.matches("store.b fixed_zp $AC, a").count(), 1);
    assert_eq!(formatted.matches("store.b fixed_zp $AD, a").count(), 1);
    assert!(formatted.contains("store_indirect (zp$AC),y+0 #$0011"));
    assert!(formatted.contains("a =.b load_indirect (zp$AC),y+0"));
    assert!(formatted.contains("store.b global g2+0, a"));
    assert!(!formatted.contains("spill sp"));
    assert_eq!(
        bytes
            .windows(2)
            .filter(|bytes| *bytes == [0x85, 0xAC])
            .count(),
        1
    );
    assert_eq!(
        bytes
            .windows(2)
            .filter(|bytes| *bytes == [0x85, 0xAD])
            .count(),
        1
    );
}

#[test]
fn lib_like_pointer_machine_block_fixtures_materialize_and_emit() {
    for fixture in [
        "range_like_pointer_out.act",
        "ord_like_machine_menu.act",
        "popup_like_machine_menu_range.act",
    ] {
        let (formatted, bytes) = compile_materialized_mir6502_fixture(fixture);
        assert!(
            formatted.contains("machine m"),
            "{fixture} should include embedded machine-block data"
        );
        assert!(
            formatted.contains("load_indirect") || formatted.contains("store_indirect"),
            "{fixture} should exercise pointer dereference materialization"
        );
        assert!(!bytes.is_empty(), "{fixture} should emit object bytes");
    }
}

#[test]
fn zero_index_pointer_read_does_not_materialize_zero_offset_word() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("array_param_string_length_loop.act");

    assert!(formatted.contains("load_indirect (zp$AC),y+0"));
    assert!(!formatted.contains("advance (zp$AC),y += #$00*1"));
    assert!(
        !bytes
            .windows(4)
            .any(|bytes| bytes == [0xA9, 0x00, 0x69, 0x00])
    );
    assert!(
        bytes
            .windows(4)
            .any(|bytes| bytes == [0x85, 0xAD, 0xA0, 0x00])
    );
    assert!(bytes.windows(2).any(|bytes| bytes == [0xB1, 0xAC]));
}

#[test]
fn byte_index_pointer_read_uses_y_instead_of_generic_word_offset() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("unsized_byte_array_dynamic_read.act");

    assert!(formatted.contains("y =.b load"));
    assert!(formatted.contains("a =.b load $0580[y]"));
    assert!(!formatted.contains("advance (zp$AC),y += a*1"));
    assert!(
        !bytes
            .windows(4)
            .any(|bytes| bytes == [0xA9, 0x00, 0x69, 0x00])
    );
    assert!(!bytes.windows(2).any(|bytes| bytes == [0x85, 0xAE]));
    assert!(!bytes.windows(2).any(|bytes| bytes == [0x85, 0xAF]));
    assert!(bytes.windows(3).any(|bytes| bytes == [0xB9, 0x80, 0x05]));
}

#[test]
fn array_param_byte_index_uses_y_instead_of_generic_word_offset() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("array_param_byte_index_loop.act");

    assert!(formatted.contains("y =.b load local l0+0"));
    assert!(formatted.contains("a =.b load (fixed_zp $AC),y"));
    assert!(!formatted.contains("advance (zp$AC),y += a*1"));
    assert!(!bytes.windows(2).any(|bytes| bytes == [0x85, 0xAE]));
    assert!(!bytes.windows(2).any(|bytes| bytes == [0x85, 0xAF]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0xB1, 0xAC]));
}

#[test]
fn pointer_scratch_avoids_source_owned_zero_page_globals() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("zero_page_global_pointer_scratch_collision.act");

    assert!(formatted.contains("global g0 i: byte absolute $00E0"));
    assert!(formatted.contains("global g1 screen: word absolute $00E6"));
    let uses_dedicated_scratch = formatted.contains("load_indirect (zp$AC),y+0");
    let uses_pointer_home = formatted.contains("load_indirect (zp$E8),y+0");
    assert!(
        uses_dedicated_scratch || uses_pointer_home,
        "pointer read should use either the dedicated scratch pair or the pointer's zero-page home:\n{formatted}"
    );
    if uses_dedicated_scratch {
        assert!(formatted.contains("store.b fixed_zp $AC, a"));
        assert!(formatted.contains("store.b fixed_zp $AD, a"));
    }
    assert!(!formatted.contains("fixed_zp $E0"));
    assert!(!formatted.contains("fixed_zp $E1"));
    assert!(!formatted.contains("fixed_zp $E6"));
    assert!(!formatted.contains("fixed_zp $E7"));
    assert!(
        bytes
            .windows(2)
            .any(|bytes| bytes == [0xB1, 0xAC] || bytes == [0xB1, 0xE8])
    );
}

#[test]
fn address_of_local_materializes_directly_to_word_store_home() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("address_of_local.act");

    assert!(!formatted.contains("lea local"));
    assert!(!formatted.contains(" v"));
    assert!(formatted.contains("store.b global g0+0, a"));
    assert!(formatted.contains("store.b global g0+1, a"));
    assert!(bytes.windows(2).any(|bytes| bytes == [0xA9, 0x02]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0xA9, 0x30]));
}

#[test]
fn local_scalar_numeric_initializers_bind_absolute_addresses() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("local_absolute_byte_aliases.act");

    assert!(formatted.contains("store.b $02BE, a"));
    assert!(formatted.contains("store.b $02D9, a"));
    assert!(formatted.contains("store.b $02DA, a"));
    assert!(!formatted.contains("local l0"));
    assert!(!formatted.contains("local l1"));
    assert!(!formatted.contains("local l2"));
    assert!(
        bytes
            .windows(5)
            .any(|bytes| bytes == [0xA9, 0x40, 0x8D, 0xBE, 0x02])
    );
    assert!(
        bytes
            .windows(5)
            .any(|bytes| bytes == [0xA9, 0x0E, 0x8D, 0xD9, 0x02])
    );
    assert!(
        bytes
            .windows(5)
            .any(|bytes| bytes == [0xA9, 0x03, 0x8D, 0xDA, 0x02])
    );
}

#[test]
fn local_card_alias_over_byte_pair_shares_storage() {
    let (_formatted, bytes) =
        compile_materialized_mir6502_fixture("local_card_alias_over_byte_pair.act");

    assert!(
        bytes
            .windows(5)
            .any(|bytes| bytes == [0xA9, 0x34, 0x8D, 0x02, 0x30])
    );
    assert!(
        bytes
            .windows(5)
            .any(|bytes| bytes == [0xA9, 0x12, 0x8D, 0x03, 0x30])
    );
    assert!(
        bytes
            .windows(6)
            .any(|bytes| bytes == [0xAD, 0x02, 0x30, 0x8D, 0x00, 0x30])
    );
    assert!(
        bytes
            .windows(6)
            .any(|bytes| bytes == [0xAD, 0x03, 0x30, 0x8D, 0x01, 0x30])
    );
}

#[test]
fn static_string_literals_are_length_prefixed_for_call_abi() {
    let (_formatted, bytes) = compile_materialized_mir6502_fixture("os_call_opaque_barrier.act");

    assert!(
        bytes
            .windows(6)
            .any(|bytes| bytes == [0x05, b'R', b'E', b'A', b'D', b'Y'])
    );
}

#[test]
fn byte_multiply_into_word_store_keeps_runtime_high_result() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("dynamic_byte_multiply_card_result.act");

    assert!(formatted.contains("store.b fixed_zp $84, a"));
    assert!(formatted.contains("store.b fixed_zp $85, a"));
    assert!(formatted.contains("helper mul args=[]"));
    assert!(formatted.contains("store.b global g2+0, a"));
    assert_spilled_x_is_reloaded_into_a(&formatted);
    assert!(formatted.contains("store.b global g2+1, a"));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x85, 0x84]));
}

#[test]
fn byte_multiply_subtract_into_word_store_keeps_runtime_high_result() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("dynamic_byte_multiply_card_sub_result.act");

    assert!(formatted.contains("helper mul args=[]"));
    assert_spilled_x_is_reloaded_into_a(&formatted);
    assert!(formatted.contains("a =.b a sub #$00 carry_in=previous"));
    assert!(formatted.contains("store.b global g2+0, a"));
    assert!(formatted.contains("store.b global g2+1, a"));
    assert!(
        !bytes
            .windows(5)
            .any(|bytes| bytes == [0xA9, 0x00, 0x8D, 0x03, 0x30]),
        "CARD high byte must not be forced to zero after multiply/subtract"
    );
}

#[test]
fn byte_multiply_word_call_arg_keeps_runtime_high_result() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("dynamic_byte_multiply_card_arg.act");

    assert!(formatted.contains("helper mul args=[]"));
    assert!(formatted.contains("call r0 args=[a.b -> a, x.b -> x]"));
    assert!(
        !formatted.contains("helper mul args=[] result=- effects=opaque,stack=?,reads=unknown,writes=unknown,clobbers=a|x|y|flags\n  x =.b #0\n  call r0"),
        "CARD multiply high byte must not be replaced with a zero before the word argument call:\n{formatted}"
    );
    assert!(formatted.contains("store.b fixed_zp $A0, a"));
    assert!(formatted.contains("store.b fixed_zp $A1, a"));
    assert!(formatted.contains("store.b global g3+0, a"));
    assert!(formatted.contains("store.b global g3+1, a"));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x85, 0x84]));
}

#[test]
fn byte_pointer_param_read_modify_write_preserves_pointer_for_store() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("byte_pointer_param_read_modify_write.act");

    assert!(formatted.contains("a =.b load param p0+0\n  store.b fixed_zp $AC, a"));
    assert!(formatted.contains("a =.b load param p0+1\n  store.b fixed_zp $AD, a"));
    assert!(formatted.contains("a =.b a sub #$01 carry_in=set carry_out=ignore"));
    assert!(formatted.contains("a =.b a add #$01 carry_in=clear carry_out=ignore"));
    assert!(formatted.contains("store_indirect (zp$AC),y+0 a"));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x85, 0xAC]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x85, 0xAD]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x91, 0xAC]));
}

#[test]
fn sargs_address_of_byte_param_uses_final_frame_address() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("sargs_address_of_byte_param.act");

    assert_sargs_address_of_c(&formatted, &bytes, 0x08, 0x0A);
}

#[test]
fn sargs_address_of_byte_param_accounts_for_initialized_global_size() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("sargs_address_of_byte_param_after_string_global.act");

    assert_sargs_address_of_c(&formatted, &bytes, 0x14, 0x16);
}

#[test]
fn sargs_address_of_byte_param_accounts_for_initialized_local_size() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("sargs_address_of_byte_param_after_local_init.act");

    assert_sargs_address_of_c(&formatted, &bytes, 0x14, 0x16);
}

fn assert_sargs_address_of_c(formatted: &str, bytes: &[u8], frame_low: u8, c_low: u8) {
    assert!(!formatted.contains("lea param"));
    assert!(formatted.contains("machine m0 items=[<a >a $03]"));
    assert!(formatted.contains("a =.b storage_addr_lo param p2+0"));
    assert!(formatted.contains("x =.b storage_addr_hi param p2+0"));
    assert!(
        bytes
            .windows(6)
            .any(|bytes| bytes == [0x20, 0xF5, 0xA0, frame_low, 0x30, 0x03])
    );
    assert!(
        bytes
            .windows(5)
            .any(|bytes| bytes == [0xA9, c_low, 0xA2, 0x30, 0x20])
    );
}

#[test]
fn pointer_backed_global_array_assignment_passes_low_and_high_call_arg() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("global_array_pointer_cell_call_arg.act");

    assert!(formatted.contains("store.b global g0+0, a"));
    assert!(formatted.contains("store.b global g0+1, a"));
    assert!(formatted.contains("a =.b load global g0+0"));
    assert!(formatted.contains("x =.b load global g0+1"));
    assert!(!formatted.contains("x =.b load global g0+0"));
    assert!(bytes.windows(3).any(|bytes| bytes == [0x8D, 0x00, 0x30]));
    assert!(bytes.windows(3).any(|bytes| bytes == [0x8D, 0x01, 0x30]));
    assert!(bytes.windows(3).any(|bytes| bytes == [0xAE, 0x01, 0x30]));
}

#[test]
fn local_array_pointer_cell_values_load_stored_pointer_for_index_and_assignment() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("local_array_pointer_cell_value.act");

    assert!(formatted.contains("inc.w local l1+0"));
    assert!(formatted.contains("a =.b #7\n  store_indirect (zp$AC),y+0 a"));
    assert!(formatted.contains("a =.b load local l1+0\n  store.b global g0+0, a"));
    assert!(formatted.contains("a =.b load local l1+1\n  store.b global g0+1, a"));
    assert!(formatted.contains("a =.b load global g0+0"));
    assert!(formatted.contains("x =.b load global g0+1"));
    assert!(!formatted.contains("store.b local l1+0, #$07"));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x91, 0xAC]));
    assert!(bytes.windows(3).any(|bytes| bytes == [0x8D, 0x01, 0x30]));
}

#[test]
fn word_store_restores_low_accumulator_before_machine_block() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("word_store_machine_block_accumulator.act");

    assert!(formatted.contains("store.b $0087, a\n  a =.b load $0086\n  machine"));
    assert!(
        bytes
            .windows(4)
            .any(|bytes| bytes == [0x85, 0x87, 0xA5, 0x86])
    );
    assert!(bytes.windows(3).any(|bytes| bytes == [0x9D, 0x44, 0x03]));
}

#[test]
fn local_card_array_initialized_from_routine_symbol_is_pointer_backed() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("local_card_array_routine_table.act");

    assert!(formatted.contains("local l0 init routine_address r1 size=2"));
    assert!(formatted.contains("a =.b load local l0+0"));
    assert!(formatted.contains("a =.b load_indirect (zp$AC),y+0"));
    assert!(!formatted.contains("local l0 init zero_fill=2"));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x05, 0x30]));
    assert!(bytes.windows(3).any(|bytes| bytes == [0x6C, 0x02, 0x30]));
}

#[test]
fn inline_card_array_pointer_assignment_uses_storage_address_as_index_base() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("inline_card_array_pointer_assignment.act");

    assert!(!formatted.contains("a =.b load local l1+0\n  store.b fixed_zp $AC, a"));
    assert!(!formatted.contains("a =.b load local l1+1\n  store.b fixed_zp $AD, a"));
    assert!(formatted.contains("store.b global g0+0, a"));
    assert!(formatted.contains("store.b global g0+1, a"));
    assert!(
        !bytes
            .windows(10)
            .any(|bytes| bytes == [0xAD, 0x06, 0x30, 0x85, 0xAC, 0xAD, 0x07, 0x30, 0x85, 0xAD])
    );
    assert!(!bytes.is_empty());
}

#[test]
fn array_param_compound_decrement_updates_pointer_high_byte() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("array_param_pointer_decrement.act");

    assert!(formatted.contains("dec.w param p0+0"));
    assert!(formatted.contains("a =.b load param p0+0"));
    assert!(formatted.contains("a =.b load param p0+1"));
    assert!(
        bytes
            .windows(8)
            .any(|bytes| bytes == [0xD0, 0x03, 0xCE, 0x02, 0x30, 0xCE, 0x01, 0x30])
    );
}

#[test]
fn unsized_local_array_symbol_decays_to_pointer_cell_value() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("unsized_local_array_pointer_cell.act");

    assert!(formatted.contains("a =.b load local l1+0\n  store.b global g0+0, a"));
    assert!(formatted.contains("a =.b load local l1+1\n  store.b global g0+1, a"));
    assert!(formatted.contains("add_byte_to_word local l1+0 #$0A"));
    assert!(formatted.contains("inc.w local l1+0"));
    assert!(formatted.contains("add_byte_to_word local l1+0 #$09"));
    assert!(!formatted.contains("addr local l1"));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x69, 0x0A]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x69, 0x09]));
}

#[test]
fn dynamic_inline_byte_read_materializes_to_absolute_y() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("byte_array_dynamic_read.act");

    assert!(formatted.contains("y =.b load global g1+0"));
    assert!(formatted.contains("a =.b load global g0+0[y]"));
    assert!(!formatted.contains("computed"));
    assert!(!formatted.contains(" v"));
    assert!(bytes.windows(3).any(|bytes| bytes == [0xAC, 0x04, 0x30]));
    assert!(bytes.windows(3).any(|bytes| bytes == [0xB9, 0x00, 0x30]));
}

#[test]
fn dynamic_inline_byte_write_materializes_to_absolute_y() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("byte_array_dynamic_write.act");

    assert!(formatted.contains("y =.b load global g1+0"));
    assert!(formatted.contains("store.b global g0+0[y], a"));
    assert!(!formatted.contains("computed"));
    assert!(!formatted.contains(" v"));
    assert!(bytes.windows(3).any(|bytes| bytes == [0xAC, 0x04, 0x30]));
    assert!(bytes.windows(3).any(|bytes| bytes == [0x99, 0x00, 0x30]));
}

#[test]
fn unsized_dynamic_word_read_materializes_to_indirect_byte_lanes() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("unsized_card_array_dynamic_read.act");

    assert!(formatted.contains("materialize_indexed (zp$AC),scaled_y <- #$0600 + a*2"));
    assert!(formatted.contains("load_indirect (zp$AC),scaled_y+0"));
    assert!(formatted.contains("load_indirect (zp$AC),scaled_y+1"));
    assert!(formatted.contains("store.b global g2+0, a"));
    assert!(formatted.contains("store.b global g2+1, a"));
    assert!(!formatted.contains("load *"));
    assert!(!formatted.contains("computed"));
    assert!(!formatted.contains(" v"));
    assert!(bytes.windows(1).any(|bytes| bytes == [0x0A]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0xB1, 0xAC]));
}

#[test]
fn descriptor_dynamic_word_read_materializes_to_indirect_byte_lanes() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("card_array_dynamic_read.act");

    assert!(formatted.contains("materialize_indexed (zp$AC),scaled_y"));
    assert!(formatted.contains("+ a*2"));
    assert!(formatted.contains("load_indirect (zp$AC),scaled_y+0"));
    assert!(formatted.contains("load_indirect (zp$AC),scaled_y+1"));
    assert!(formatted.contains("store.b global g2+0, a"));
    assert!(formatted.contains("store.b global g2+1, a"));
    assert!(!formatted.contains("load *"));
    assert!(!formatted.contains("computed"));
    assert!(!formatted.contains(" v"));
    assert!(bytes.windows(1).any(|bytes| bytes == [0x0A]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0xB1, 0xAC]));
}

#[test]
fn unsized_dynamic_word_write_materializes_to_indirect_byte_lanes() {
    let (formatted, bytes) =
        compile_materialized_mir6502_fixture("unsized_card_array_dynamic_write.act");

    assert!(formatted.contains("materialize_indexed (zp$AC),scaled_y <- #$0600 + a*2"));
    assert!(formatted.contains("store_indirect (zp$AC),scaled_y+0 a"));
    assert!(formatted.contains("store_indirect (zp$AC),scaled_y+1 a"));
    assert!(!formatted.contains("store.w *"));
    assert!(!formatted.contains("computed"));
    assert!(!formatted.contains(" v"));
    assert!(bytes.windows(1).any(|bytes| bytes == [0x0A]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x91, 0xAC]));
}

#[test]
fn descriptor_dynamic_word_write_materializes_to_indirect_byte_lanes() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("card_array_dynamic_write.act");

    assert!(formatted.contains("materialize_indexed (zp$AC),scaled_y"));
    assert!(formatted.contains("+ a*2"));
    assert!(formatted.contains("store_indirect (zp$AC),scaled_y+0 a"));
    assert!(formatted.contains("store_indirect (zp$AC),scaled_y+1 a"));
    assert!(!formatted.contains("store.w computed"));
    assert!(!formatted.contains("computed"));
    assert!(!formatted.contains(" v"));
    assert!(bytes.windows(1).any(|bytes| bytes == [0x0A]));
    assert!(bytes.windows(2).any(|bytes| bytes == [0x91, 0xAC]));
}

#[test]
fn constant_index_reads_materialize_to_direct_offsets() {
    for fixture in [
        "byte_array_const_read.act",
        "card_array_const_read.act",
        "string_index_read.act",
    ] {
        let (formatted, bytes) = compile_materialized_mir6502_fixture(fixture);

        assert!(
            !formatted.contains("spill sp"),
            "{fixture} copied through spill storage:\n{formatted}"
        );
        assert!(
            !formatted.contains("lea "),
            "{fixture} kept address scaffolding:\n{formatted}"
        );
        assert!(formatted.contains("load global"));
        assert!(!bytes.is_empty());
    }
}

#[test]
fn descriptor_backed_constant_word_reads_materialize_to_indirect_lanes() {
    for fixture in [
        "descriptor_backing_reference.act",
        "local_initialized_array_storage.act",
    ] {
        let (formatted, bytes) = compile_materialized_mir6502_fixture(fixture);

        assert!(
            formatted.contains("load_indirect (zp$AC),y+"),
            "{fixture} did not load through its descriptor backing:\n{formatted}"
        );
        assert!(
            !formatted.contains(" v"),
            "{fixture} leaked virtual temps:\n{formatted}"
        );
        assert!(
            bytes.windows(2).any(|bytes| bytes == [0xB1, 0xAC]),
            "{fixture} did not emit LDA ($AC),Y"
        );
    }
}

#[test]
fn record_field_reads_materialize_to_direct_offsets() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("record_field_read.act");

    assert!(!formatted.contains("spill sp"));
    assert!(!formatted.contains("lea "));
    assert!(formatted.contains("load global g1+0"));
    assert!(formatted.contains("store.b global g2+0"));
    assert!(!bytes.is_empty());
}

#[test]
fn pointer_equality_branch_materializes_to_byte_lane_flag_branches() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("pointer_compare.act");

    assert!(formatted.contains("cmp_word_lo"));
    assert!(formatted.contains("cmp_word_hi"));
    assert!(formatted.contains("branch flag z_clear"));
    assert!(formatted.contains("branch flag z_set"));
    assert!(!formatted.contains("cmp.w"));
    assert!(!formatted.contains("branch bool"));
    assert!(!formatted.contains(" v"));
    assert!(bytes.windows(2).any(|bytes| bytes[0] == 0xD0));
    assert!(bytes.windows(2).any(|bytes| bytes[0] == 0xF0));
}

#[test]
fn unsigned_word_relational_branch_materializes_high_then_low() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("card_compare.act");

    assert!(formatted.contains("cmp_word_hi_lt"));
    assert!(formatted.contains("cmp_word_hi_eq"));
    assert!(formatted.contains("cmp_word_lo_lt"));
    assert!(formatted.contains("branch flag c_clear"));
    assert!(formatted.contains("branch flag z_set"));
    assert!(!formatted.contains("cmp.w"));
    assert!(!formatted.contains("branch bool"));
    assert!(!formatted.contains(" v"));
    assert!(bytes.windows(2).any(|bytes| bytes[0] == 0x90));
}

#[test]
fn signed_int_relational_branch_materializes_sign_split() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("signed_int_compare.act");

    assert!(formatted.contains("cmp_i16_sub"));
    assert!(formatted.contains("branch flag v_set"));
    assert!(formatted.contains("branch flag n_clear"));
    assert!(formatted.contains("branch flag n_set"));
    assert!(!formatted.contains("cmp.w"));
    assert!(!formatted.contains("branch bool"));
    assert!(!formatted.contains(" v0"));
    assert!(
        bytes
            .windows(2)
            .any(|bytes| matches!(bytes[0], 0x50 | 0x70))
    );
    assert!(bytes.windows(2).any(|bytes| bytes[0] == 0x10));
    assert!(bytes.windows(2).any(|bytes| bytes[0] == 0x30));
}

#[test]
fn short_circuit_and_or_materialize_to_control_flow() {
    let (and_formatted, and_bytes) = compile_materialized_mir6502_fixture("short_circuit_and.act");
    let (or_formatted, or_bytes) = compile_materialized_mir6502_fixture("short_circuit_or.act");

    for formatted in [&and_formatted, &or_formatted] {
        assert_eq!(formatted.matches("branch fused").count(), 2);
        assert_eq!(formatted.matches("z_clear").count(), 2);
        assert!(!formatted.contains(" and "));
        assert!(!formatted.contains(" or "));
        assert!(!formatted.contains("branch bool"));
        assert!(!formatted.contains(" v"));
    }
    assert!(
        and_bytes
            .windows(2)
            .any(|bytes| matches!(bytes[0], 0xD0 | 0xF0))
    );
    assert!(
        or_bytes
            .windows(2)
            .any(|bytes| matches!(bytes[0], 0xD0 | 0xF0))
    );
}

#[test]
fn compare_or_with_call_uses_conditional_rhs_block() {
    let (formatted, _) = compile_materialized_mir6502_fixture("compare_or_with_call.act");

    let call = formatted.find("call r0").expect("conditional call");
    let second_branch = formatted
        .match_indices("branch fused")
        .nth(1)
        .map(|(offset, _)| offset)
        .expect("two prefix branches");
    assert!(call > second_branch, "{formatted}");
    assert_eq!(formatted.matches("branch fused").count(), 3);
    assert!(!formatted.contains(" or "));
    assert!(!formatted.contains("a =.b a or"));
}

#[test]
fn indirect_call_targets_materialize_to_callable_home() {
    for fixture in [
        "indirect_proc_call.act",
        "indirect_func_call_byte.act",
        "indirect_func_call_word.act",
        "callable_param_forwarding.act",
    ] {
        let (formatted, bytes) = compile_materialized_mir6502_fixture(fixture);

        assert!(
            formatted.contains("call indirect word(*fixed_zp $E4, *fixed_zp $E5).w"),
            "{fixture} did not materialize its indirect call target home:\n{formatted}"
        );
        assert!(
            !formatted.contains("call indirect v"),
            "{fixture} leaked a virtual temp indirect call target:\n{formatted}"
        );
        assert!(
            bytes.windows(3).any(|bytes| bytes == [0x6C, 0xE4, 0x00]),
            "{fixture} did not emit JMP ($00E4)"
        );
        assert!(
            bytes.windows(1).any(|bytes| bytes == [0x48]),
            "{fixture} did not push an indirect-call return address"
        );
    }
}

#[test]
fn routine_address_values_emit_as_object_labels() {
    let (formatted, bytes) = compile_materialized_mir6502_fixture("indirect_proc_call.act");
    let helper_addr = CODE_ORIGIN + 2;
    let helper_addr_low = (helper_addr & 0x00FF) as u8;
    let helper_addr_high = (helper_addr >> 8) as u8;

    assert!(formatted.contains("routine_addr_lo r0"));
    assert!(formatted.contains("routine_addr_hi r0"));
    assert!(
        bytes
            .windows(2)
            .any(|bytes| bytes == [0xA9, helper_addr_low])
    );
    assert!(
        bytes
            .windows(2)
            .any(|bytes| bytes == [0xA9, helper_addr_high])
    );
    assert!(bytes.windows(3).any(|bytes| bytes == [0x6C, 0xE4, 0x00]));
}

#[test]
fn final_gap_cluster_fixtures_materialize_and_emit() {
    for fixture in [
        "unsized_byte_array_dynamic_read.act",
        "unsized_byte_array_dynamic_write.act",
        "local_card_array_dynamic_read.act",
        "machine_block_byte_stream.act",
        "machine_block_label_ref.act",
        "machine_block_global_ref.act",
        "pass_byte_array_param.act",
        "pass_card_array_param.act",
        "for_loop_word.act",
        "global_scalars_layout.act",
        "record_field_byte_store.act",
    ] {
        let (formatted, bytes) = compile_materialized_mir6502_fixture(fixture);

        assert!(
            !formatted.contains(" v"),
            "{fixture} leaked virtual temps:\n{formatted}"
        );
        assert!(
            !formatted.contains("computed"),
            "{fixture} leaked computed-index pseudo ops:\n{formatted}"
        );
        assert!(
            !formatted.contains("lea "),
            "{fixture} leaked address pseudo ops:\n{formatted}"
        );
        assert!(!bytes.is_empty(), "{fixture} emitted no object bytes");
    }
}

#[test]
fn machine_block_byte_stream_fixture_emits_width_rule_bytes() {
    let (_formatted, bytes) = compile_materialized_mir6502_fixture("machine_block_byte_stream.act");
    let expected = [
        0x34, 0x48, 0x03, 0xF8, 0xF2, 0xFF, 0xFF, 0xE4, 0xE5, 0xE4, 0x00,
    ];

    assert!(
        bytes
            .windows(expected.len())
            .any(|window| window == expected),
        "expected machine byte stream {expected:02X?} in {bytes:02X?}"
    );
}

#[test]
fn stress_pointers_materialize_index_address_consumers() {
    let Err(diagnostics) = verify_materialized_mir6502_stress_fixture("pointers.act") else {
        return;
    };
    let messages = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect::<Vec<_>>();

    for forbidden in [
        "computed index addresses must be materialized before emission",
        "dynamic word index addresses must be materialized before emission",
        "dynamic pointer word index addresses must be materialized before emission",
    ] {
        assert!(
            !messages.iter().any(|message| message.contains(forbidden)),
            "unexpected address diagnostic `{forbidden}` in {messages:#?}"
        );
    }
}

#[test]
fn stress_pointers_byte_temp_word_pointer_store_zero_extends() {
    verify_materialized_mir6502_stress_fixture("pointers.act")
        .expect("pointers stress fixture should materialize after byte-temp word pointer store");
}

#[test]
fn stress_zero_page_word_add_byte_temp_rhs_materializes() {
    verify_materialized_mir6502_stress_fixture("zero_page.act")
        .expect("zero_page stress fixture should materialize word add with byte temp rhs");
}

#[test]
fn stress_pointer_torture_word_neg_pointer_store_materializes() {
    verify_materialized_mir6502_stress_fixture("pointer_torture.act")
        .expect("pointer_torture stress fixture should materialize word neg pointer store");
}

#[test]
fn stress_control_flow_word_compare_temps_materialize() {
    verify_materialized_mir6502_stress_fixture("control_flow.act")
        .expect("control_flow stress fixture should materialize standalone word compare temps");
}

fn compile_materialized_mir6502_fixture(name: &str) -> (String, Vec<u8>) {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("mir6502")
        .join(name);
    let loaded = load_program_with_expanded_source(&fixture)
        .unwrap_or_else(|err| panic!("load {}: {err:?}", fixture.display()));
    let model = analyze(&loaded.program)
        .unwrap_or_else(|err| panic!("analyze {}: {err:?}", fixture.display()));
    let semir = ir::lower_program(&loaded.program, &model);
    let nir_program = nir::lower_program(&semir);
    nir::verify_program(&nir_program)
        .unwrap_or_else(|err| panic!("verify NIR for {}: {err:?}", fixture.display()));

    let mir = mir6502::lower_program(&nir_program)
        .unwrap_or_else(|err| panic!("lower MIR6502 for {}: {err:?}", fixture.display()));
    let materialized = mir6502::materialize_program(mir, &mir6502::Mir6502Config::default())
        .unwrap_or_else(|err| panic!("materialize MIR6502 for {}: {err:?}", fixture.display()));
    mir6502::verify_program(&materialized, mir6502::MirPhase::PreEmission).unwrap_or_else(|err| {
        panic!(
            "verify materialized MIR6502 for {}: {err:?}",
            fixture.display()
        )
    });

    let formatted = mir6502::format_program(&materialized);
    let output = mir6502::generate_output(&nir_program, CODE_ORIGIN)
        .unwrap_or_else(|err| panic!("emit MIR6502 for {}: {err:?}", fixture.display()));
    (formatted, output.bytes)
}

fn assert_spilled_x_is_reloaded_into_a(formatted: &str) {
    let home = formatted
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("store.b spill ")
                .and_then(|rest| rest.strip_suffix(", x"))
        })
        .expect("runtime high byte is preserved from X in a spill");
    assert!(
        formatted
            .lines()
            .any(|line| line.trim() == format!("a =.b load spill {home}")),
        "runtime high-byte spill {home} must be reloaded into A"
    );
}

fn verify_materialized_mir6502_stress_fixture(
    name: &str,
) -> Result<(), Vec<mir6502::MirDiagnostic>> {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join("stress")
        .join(name);
    let loaded = load_program_with_expanded_source(&fixture)
        .unwrap_or_else(|err| panic!("load {}: {err:?}", fixture.display()));
    let model = analyze(&loaded.program)
        .unwrap_or_else(|err| panic!("analyze {}: {err:?}", fixture.display()));
    let semir = ir::lower_program(&loaded.program, &model);
    let nir_program = nir::lower_program(&semir);
    nir::verify_program(&nir_program)
        .unwrap_or_else(|err| panic!("verify NIR for {}: {err:?}", fixture.display()));

    let mir = mir6502::lower_program(&nir_program)
        .unwrap_or_else(|err| panic!("lower MIR6502 for {}: {err:?}", fixture.display()));
    let materialized = mir6502::materialize_program(mir, &mir6502::Mir6502Config::default())
        .unwrap_or_else(|err| panic!("materialize MIR6502 for {}: {err:?}", fixture.display()));
    mir6502::verify_program(&materialized, mir6502::MirPhase::PreEmission)
}
