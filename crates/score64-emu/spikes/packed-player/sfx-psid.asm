// PSID wrapper (KickAssembler 5.25): a gt2reloc -D1 tune plus up to three sound effects, fired on voice 3 every 37
// frames in rotation, the same schedule as sfx-driver.asm and `score64-emu render --effect ...`. Lets any SID player
// (sidplayfp, VICE vsid) play the music with its effects, for listening on reSIDfp (#18).
//
//   java -jar KickAss.jar sfx-psid.asm -binfile -o wrapper.bin :tune=tune.sid :fx1=a.bin [:fx2=b.bin :fx3=c.bin]
//   python3 make_sfx_psid.py wrapper.bin out.sid "<name>" <subtunes>
//
// Layout: wrapper code at $0E00, effects at $0E80/$0F00/$0F80 (128 bytes each), the player at $1000 (gt2reloc's
// default), so the PSID's data is one contiguous block.

.const PLAYER       = $1000
.const SFX          = PLAYER + 6
.const FX_BASE      = $0e80
.const FX_STRIDE    = $80
.const FX_EVERY     = 37
.const FX_CHANNEL   = 14        // voice 3

.var fxFiles = List()
.for (var i = 1; i <= 3; i++) {
    .if (cmdLineVars.containsKey("fx" + i)) .eval fxFiles.add(cmdLineVars.get("fx" + i))
}

* = $0e00 "Wrapper"
init:                           // $0E00, A = subtune (0-based), passed on to the player
        ldx #1
        stx first
        ldx #FX_EVERY
        stx fx_count
        ldx #0
        stx fx_index
        jmp PLAYER

* = $0e20
play:                           // $0E20, once per frame
        lda first
        beq !countdown+
        lda #0
        sta first
        jmp PLAYER + 3
!countdown:
        dec fx_count
        bne !done+
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
!done:
        jmp PLAYER + 3

first:    .byte 1
fx_count: .byte FX_EVERY
fx_index: .byte 0
fx_lo:    .fill fxFiles.size(), <(FX_BASE + i * FX_STRIDE)
fx_hi:    .fill fxFiles.size(), >(FX_BASE + i * FX_STRIDE)
.assert "wrapper fits below the effects", * <= FX_BASE, true

.for (var i = 0; i < 3; i++) {
    * = FX_BASE + i * FX_STRIDE "Effect"
    .if (i < fxFiles.size()) .import binary fxFiles.get(i)
    .if (i >= fxFiles.size()) .byte 0
    .fill FX_BASE + (i + 1) * FX_STRIDE - *, 0
}

* = PLAYER "Player"
        // PSID v2 header ($7c bytes) and the two-byte load address are skipped.
        .import binary cmdLineVars.get("tune"), $7e
