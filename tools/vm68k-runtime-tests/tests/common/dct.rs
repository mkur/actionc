//! Shared DCT capture instrumentation for VM and paired C reference tests.
fn replace_once(source: &mut String, old: &str, new: &str) {
    assert_eq!(
        source.matches(old).count(),
        1,
        "instrumentation anchor: {old}"
    );
    *source = source.replace(old, new);
}

pub fn instrument(source: &str, shaped: bool) -> String {
    let mut source = source.replace("\r\n", "\n");
    replace_once(
        &mut source,
        "INT result\n",
        "INT result\n\
        BYTE testCommand,testShift\n\
        LONGINT ARRAY testInput(64),testInitial(64),testRows(64)\n\
        PROC CaptureRows()\n\
          BYTE i\n\
          FOR i=0 TO 63 DO testRows(i)=block(i) OD\n\
        RETURN\n",
    );
    replace_once(
        &mut source,
        "  ; Pass 2: eight values per column, eight elements apart.",
        "  CaptureRows()\n  ; Pass 2: eight values per column, eight elements apart.",
    );
    replace_once(
        &mut source,
        "PROC Main()\n  Init()\n  Dct()\n  result=CheckResult()\nRETURN\n",
        "PROC Main()\n\
          BYTE i\n\
          IF testCommand=0 THEN Init()\n\
          ELSE FOR i=0 TO 63 DO block(i)=testInput(i) OD FI\n\
          FOR i=0 TO 63 DO testInitial(i)=block(i) OD\n\
          IF testCommand=2 THEN\n\
            FOR i=0 TO 63 DO block(i)=Descale(block(i),testShift) OD\n\
          ELSE Dct() FI\n\
          result=CheckResult()\n\
        RETURN\n",
    );
    if shaped {
        // Only generated capture/driver accesses use this spelling in the
        // shaped source. Keep the algorithm itself in two coordinates.
        assert_eq!(source.matches("block(i)").count(), 5);
        source = source.replace("block(i)", "testFlat(i)");
        replace_once(
            &mut source,
            "INT result\n",
            "INT result\nLONGINT ARRAY testFlat\n",
        );
        replace_once(
            &mut source,
            "IF testCommand=0 THEN Init()",
            "testFlat=block\nIF testCommand=0 THEN Init()",
        );
    }
    source
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_dct_instrumentation_accepts_both_host_line_endings() {
        for (source, shaped) in [
            (
                include_str!("../../../../fixtures/runtime/tacle/jfdctint/jfdctint.act"),
                false,
            ),
            (
                include_str!("../../../../fixtures/runtime/tacle/jfdctint/multidimensional.act"),
                true,
            ),
        ] {
            let lf = source.replace("\r\n", "\n");
            let actual = instrument(&lf, shaped);
            assert_eq!(actual, instrument(&lf.replace('\n', "\r\n"), shaped));
            assert_eq!(actual.matches("  CaptureRows()\n").count(), 1);
        }
    }
}
