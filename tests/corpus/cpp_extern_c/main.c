/* Control: the same quoted include OUTSIDE any error-recovery region. It
   resolved before the fix and must keep resolving after it. */
#include "utils.h"

int main(void) {
    return utils_run() + core_value() + vector_len();
}
