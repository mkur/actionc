/* Bare CPU image for the same origin and protected trampoline as Action!. */
OUTPUT_ARCH(m68k)
ENTRY(Main)
PHDRS {
  text PT_LOAD FLAGS(5);
  rodata PT_LOAD FLAGS(4);
  data PT_LOAD FLAGS(6);
}
SECTIONS
{
  . = 0x10000;
  .text : {
    __code_start = .;
    *(.text .text.*)
    . = ALIGN(2);
    /* Same four bytes of legal prefetch padding as native Action! images. */
    SHORT(0x4e71) SHORT(0x4e71)
    __code_end = .;
  } :text
  .rodata : ALIGN(4) {
    __rodata_start = .;
    *(.rodata .rodata.*)
    __rodata_end = .;
  } :rodata
  .data : ALIGN(4) {
    __data_start = .;
    *(.data .data.*)
    __data_end = .;
  } :data
  .bss (NOLOAD) : ALIGN(4) {
    __bss_start = .;
    *(.bss .bss.*) *(COMMON)
    __bss_end = .;
  } :data
  /DISCARD/ : { *(.comment) *(.note*) *(.eh_frame*) }
}
