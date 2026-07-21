# Page 1
        BLOCK   02
        BANK    2
        SETLOC  4000
        EBANK=  3
        COUNT*  MAIN

BASE    EQUALS  10
MASK    OCT     777
PAIR    2OCT    10
SIGNED  DEC     -2
TWICE   2DEC    11
SPACE   ERASE   2
ADDR    ADRES   BASE
CADRVAL CADR    START
ECADRVAL ECADR  START
GENVAL  GENADR  START
VECTOR  VN      1
BBANK   BBCON   START
ALIAS   =       BASE

START   TC      LOCAL
        TCF     OFFSET +2
        CAF     BASE
        TS      MASK
        TC      Q

# Interpretive transfers put their address word on the next line.
INTERP  DLOAD   CALL
        SUBROUTINE
        BZE     GOTO
        FINISH

OFFSET  TCF     FINISH-2
LOCAL   TC      FINISH
        $SUB.agc
FINISH  TC      Q
