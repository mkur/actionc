 section .text,"acrx"
 global start
start:
 jsl target
 rtl
 section .data,"adrw"
 global target
target:
 byte <target, >target, ^target
 word target
 defl target
