// Test driver for x64sc (KickAssembler 5.25): plays a gt2reloc -D1 tune and fires sound effects on the same
// schedule as `score64-emu render --effect a.bin [--effect b.bin ...] --effect-every 37 --effect-voice 3`, so the two
// SID write logs can be compared frame by frame (#20, #18).
//
//   java -jar KickAss.jar sfx-driver.asm -o driver.prg :tune=tune.sid :fx=fx.bin [:subtune=1]
//   java -jar KickAss.jar sfx-driver.asm -o driver.prg :tune=tune.sid :fx1=a.bin :fx2=b.bin :fx3=c.bin
//
// Effects rotate in the order given; the n-th is loaded at $C000 + n * $80, as in the helper. subtune is 0-based.
//
// Own raster IRQ through $FFFE with the KERNAL banked out, and the screen blanked so no bad lines steal cycles.

.const PLAYER       = $1000     // gt2reloc default (-W10)
.const INIT         = PLAYER
.const PLAY         = PLAYER + 3
.const SFX          = PLAYER + 6 // only with -D1
.const FX_ADDRESS   = $c000
.const FX_EVERY     = 37
.const FX_CHANNEL   = 14        // voice 3
.const IRQ_LINE     = $30
.const FX_STRIDE    = $80

.var fxFiles = List()
.if (cmdLineVars.containsKey("fx")) .eval fxFiles.add(cmdLineVars.get("fx"))
.for (var i = 1; i <= 3; i++) {
    .if (cmdLineVars.containsKey("fx" + i)) .eval fxFiles.add(cmdLineVars.get("fx" + i))
}
.var subtune = cmdLineVars.containsKey("subtune") ? cmdLineVars.get("subtune").asNumber() : 0

BasicUpstart2(start)

* = $0810 "Driver"
start:
        sei
        lda #$7f
        sta $dc0d               // no CIA interrupts
        sta $dd0d
        lda $dc0d
        lda $dd0d
        lda #$35                // RAM, I/O, no BASIC or KERNAL
        sta $01
        lda #<irq
        sta $fffe
        lda #>irq
        sta $ffff
        lda #<nmi
        sta $fffa
        lda #>nmi
        sta $fffb

        lda #$0b                // screen off: no bad lines
        sta $d011
        lda #IRQ_LINE
        sta $d012

        lda #subtune
        jsr INIT

        lda #$01
        sta $d01a               // raster IRQ on
        sta $d019
        cli
        jmp *

irq:
        pha
        txa
        pha
        tya
        pha
        lda #$ff
        sta $d019

        lda first
        beq !countdown+
        lda #0
        sta first
        jmp !play+
!countdown:
        dec fx_count
        bne !play+
        lda #FX_EVERY
        sta fx_count
        ldx fx_index
        ldy fx_hi,x
        lda fx_lo,x
        pha
        inx
        cpx #fxFiles.size()
        bne !+
        ldx #0
!:      stx fx_index
        pla
        ldx #FX_CHANNEL
        jsr SFX
!play:
        jsr PLAY

        pla
        tay
        pla
        tax
        pla
nmi:
        rti

first:    .byte 1
fx_count: .byte FX_EVERY
fx_index: .byte 0
fx_lo:    .fill fxFiles.size(), <(FX_ADDRESS + i * FX_STRIDE)
fx_hi:    .fill fxFiles.size(), >(FX_ADDRESS + i * FX_STRIDE)

* = PLAYER "Player"
        // PSID v2 header ($7c bytes) and the two-byte load address are skipped.
        .import binary cmdLineVars.get("tune"), $7e

.for (var i = 0; i < fxFiles.size(); i++) {
    * = FX_ADDRESS + i * FX_STRIDE "Effect"
        .import binary fxFiles.get(i)
}
