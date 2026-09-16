; Freestanding Action! native-v1 context bridge. Assemble in bank zero.
; The embedding platform supplies A816_* configuration symbols listed in
; docs/MIR65816_CONTEXT_INTERFACE.md. No task-selection policy lives here.
.setcpu "65816"
.smart
.include "action65816-native-v1.inc"
.export __a816_irq_v1, __a816_cop_v1, __a816_nmi_v1, __a816_restore_v1
.export __a816_yield_v1, __a816_task_return_v1, __a816_terminal_v1

.assert A816_IRQ_DP .mod 256 = 0, error, "IRQ direct page must be aligned"
.assert A816_IRQ_STACK_TOP .mod 2 = 0, error, "IRQ stack top must be even"
.assert A816_IRQ_STACK_TOP >= A816_IRQ_STACK_FLOOR + 6, error, "IRQ bridge requires six ordinary stack bytes"
.assert A816_IRQ_STACK_TOP < $10000, error, "IRQ stack must be in bank zero"
.assert A816_IRQ_DP <= $FF00, error, "IRQ direct page must be in bank zero"

; Check before every ordinary push. Does not change S or I; clobbers A/X/flags.
.macro check_stack amount
 .local below, good, bad
 tsc
 tax
 cmp A816_DP_STACK_CEILING_OFFSET
 bcc below
 beq below
 bra bad
below:
 sec
 sbc #amount
 bcc bad
 cmp A816_DP_STACK_FLOOR_OFFSET
 bcs good
bad:
 lda #amount
 jml A816_STACK_OVERFLOW
 good:
.endmacro

.macro save_full
 rep #$30
 pha
 phx
 phy
 phd
 phb
.endmacro

.a16
.i16
__a816_irq_v1:
 save_full
 tsc
 tax
 ldy #A816_DISPATCH_IRQ
 bra dispatch
__a816_cop_v1:
 save_full
 tsc
 tax
 ldy #A816_DISPATCH_YIELD
 dispatch:
 ; X owns saved_s until written to the IRQ call area. Nothing touches the
 ; interrupted direct-page memory. NMI can interrupt each transition.
 lda #A816_IRQ_STACK_TOP
 tcs
 lda #A816_IRQ_DP
 tcd
 sep #$20
 lda #0
 pha
 plb
 rep #$20
 cld
 tya
 sta $10                       ; reason in IRQ-owned scratch
 cmp #A816_DISPATCH_YIELD
 bne invoke
 ; COP is resumable only for signature zero from a task with I clear.
 lda a:A816_SAVED_FRAME_P_OFFSET,x
 and #4
 bne __a816_terminal_v1
 lda a:A816_SAVED_FRAME_D_OFFSET,x
 tay
 sep #$20
 lda a:A816_DP_DOMAIN_KIND_OFFSET,y
 bne __a816_terminal_v1
 rep #$20
 lda a:A816_SAVED_FRAME_PC_OFFSET,x
 dec a                         ; PC-1 within the saved program bank
 sta $00
 sep #$20
 lda a:A816_SAVED_FRAME_PBR_OFFSET,x
 sta $02
 lda [$00]
 bne __a816_terminal_v1
 rep #$20
 invoke:
 .a16
 tsc
 sec
 sbc #3                        ; (CARD saved_s, BYTE reason): L=O=3
 tcs
 txa
 sta 1,s
 sep #$20
 lda $10
 sta 3,s
 rep #$20
 jsl A816_DISPATCH
 tay
 tsc
 clc
 adc #3
 tcs
 tya
 ; The dispatch hook has published its selected task. No IRQ activation is
 ; retained. A is the complete selected saved_s, not a task-record pointer.
__a816_restore_v1:
 sei
 rep #$30
 cld
 tcs
 plb
 pld
 ply
 plx
 pla
 rti

__a816_terminal_v1:
 sei
 rep #$30
 cld
 jml A816_TERMINAL

__a816_yield_v1:
 .a16
 .i16
 check_stack 1
 php
 sep #$20
 pla
 and #4
 bne __a816_terminal_v1
 lda A816_DP_DOMAIN_KIND_OFFSET
 bne __a816_terminal_v1
 rep #$20
 cop A816_INTERRUPT_YIELD_COP_SIGNATURE
 rtl

__a816_nmi_v1:
 save_full
 ; Exactly 13 bytes including the hardware frame, no direct-page access and
 ; no dependency on DBR, D, or task/IRQ transition bookkeeping.
 sep #$20
 lda #1
 sta f:A816_NMI_ACK
 rep #$30
 plb
 pld
 ply
 plx
 pla
 rti

__a816_task_return_v1:
 .a16
 .i16
 tsc
 clc
 adc #A816_FIRST_TASK_OUTGOING_BYTES
 tcs
 check_stack 4
 tsc
 dec a
 tcs
 sep #$20
 lda #0
 sta 1,s
 rep #$20
 jsl A816_TASK_EXIT
 jml __a816_terminal_v1         ; exit is nonreturning


.export __a816_irq_save_disable_v1, __a816_irq_restore_v1
__a816_irq_save_disable_v1:
 .a16
 .i16
 check_stack 1
 php
 sei
 sep #$20
 pla
 and #4
 rep #$20
 and #$00FF
 rtl
__a816_irq_restore_v1:
 .a16
 .i16
 sep #$20
 lda 4,s
 and #4
 beq enable_irq
 rep #$20
 sei
 rtl
enable_irq:
 rep #$20
 cli
 rtl
