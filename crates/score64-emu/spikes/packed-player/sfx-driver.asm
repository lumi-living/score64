// Test driver for x64sc (KickAssembler 5.25): plays a gt2reloc -D1 tune and fires a sound effect on the same
// schedule as `score64-emu render --effect fx.bin --effect-every 37 --effect-voice 3`, so the two SID write logs can
// be compared frame by frame (#20).
//
//   java -jar KickAss.jar sfx-driver.asm -o driver.prg :tune=path/to/tune.sid :fx=path/to/fx.bin
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

        lda #0                  // subtune 1
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
        lda #<FX_ADDRESS
        ldy #>FX_ADDRESS
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

* = PLAYER "Player"
        // PSID v2 header ($7c bytes) and the two-byte load address are skipped.
        .import binary cmdLineVars.get("tune"), $7e

* = FX_ADDRESS "Effect"
        .import binary cmdLineVars.get("fx")
