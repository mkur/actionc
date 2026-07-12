; ----------------------------------------------------------------------
; Output routines
; ----------------------------------------------------------------------

PROC Print(<string>)
PROC PrintE(<string>)
PROC PrintD(BYTE channel, <string>)
PROC PrintDE(BYTE channel, <string>)

PROC PrintB(BYTE number)
PROC PrintBE(BYTE number)
PROC PrintBD(BYTE channel, BYTE number)
PROC PrintBDE(BYTE channel, BYTE number)

PROC PrintC(CARD number)
PROC PrintCE(CARD number)
PROC PrintCD(BYTE channel, CARD number)
PROC PrintCDE(BYTE channel, CARD number)

PROC PrintI(INT number)
PROC PrintIE(INT number)
PROC PrintID(BYTE channel, INT number)
PROC PrintIDE(BYTE channel, INT number)

PROC PrintF(<control string>, <data>|:, <data>:|)

PROC Put(CHAR character)
PROC PutE()
PROC PutD(BYTE channel, CHAR character)
PROC PutDE(BYTE channel)


; ----------------------------------------------------------------------
; Input routines
; ----------------------------------------------------------------------

BYTE FUNC InputB()
BYTE FUNC InputBD(BYTE channel)

CARD FUNC InputC()
CARD FUNC InputCD(BYTE channel)

INT FUNC InputI()
INT FUNC InputID(BYTE channel)

PROC InputS(<string>)
PROC InputSD(BYTE channel, <string>)
PROC InputMD(BYTE channel, <string>, BYTE max)

CHAR FUNC GetD(BYTE channel)


; ----------------------------------------------------------------------
; File manipulation routines
; ----------------------------------------------------------------------

PROC Open(BYTE channel, <filestring>, BYTE mode, aux2)
PROC Close(BYTE channel)

PROC XIO(BYTE chan, 0, cmd, aux1, aux2, <filestring>)

PROC Note(BYTE chan, CARD POINTER sector, BYTE POINTER offset)
PROC Point(BYTE chan, CARD sector, BYTE offset)


; ----------------------------------------------------------------------
; Graphics and game-controller routines
; ----------------------------------------------------------------------

PROC Graphics(BYTE mode)
PROC SetColor(BYTE register, hue, luminance)

PROC Plot(CARD col, BYTE row)
PROC DrawTo(CARD col, BYTE row)
PROC Fill(CARD col, BYTE row)
PROC Position(CARD col, BYTE row)

BYTE FUNC Locate(CARD col, BYTE row)

PROC Sound(BYTE voice, pitch, distortion, volume)
PROC SndRst()

BYTE FUNC Paddle(BYTE port)
BYTE FUNC PTrig(BYTE port)
BYTE FUNC Stick(BYTE port)
BYTE FUNC STrig(BYTE port)


; ----------------------------------------------------------------------
; String handling / conversion routines
; ----------------------------------------------------------------------

INT FUNC SCompare(<string1>, <string2>)

PROC SCopy(<dest>, <source>)
PROC SCopyS(<dest>, <source>, BYTE start, stop)
PROC SAssign(<dest>, <source>, BYTE start, stop)

PROC StrB(BYTE number, <string>)
PROC StrC(CARD number, <string>)
PROC StrI(INT number, <string>)

BYTE FUNC ValB(<string>)
CARD FUNC ValC(<string>)
INT FUNC ValI(<string>)


; ----------------------------------------------------------------------
; Miscellaneous routines
; ----------------------------------------------------------------------

BYTE FUNC Rand(BYTE range)

PROC Break()
PROC Error(BYTE errcode)

BYTE FUNC Peek(CARD address)
CARD FUNC PeekC(CARD address)

PROC Poke(CARD address, BYTE value)
PROC PokeC(CARD address, CARD value)

PROC Zero(BYTE POINTER address, CARD size)
PROC SetBlock(BYTE POINTER address, CARD size, BYTE value)
PROC MoveBlock(BYTE POINTER dest, source, CARD size)