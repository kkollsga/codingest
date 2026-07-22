SUBROUTINE TC    HELPER
        CA      BASE
        TC      BANKCALL
        CADR    HELPER
        TC      IBNKCALL
        FCADR   HELPER
        TC      POSTJUMP
        CADR    DONE
        TC      BANKJUMP
        CA      HELPER
        TC      SWCALL
DONE    TC      Q
HELPER  TC      Q
