/* The other half of the same real-world shape: a header that OPENS the
   `extern "C"` block and leaves the caller's `decls_end.h` to close it, so
   the brace is unbalanced within this translation unit. tree-sitter has no
   preprocessor and no closing brace to pair with, so the whole file below
   the guard parses as one ERROR recovery subtree — and every `#include`
   inside it used to be invisible to the router. */
#ifndef DECLS_BEGIN_H
#define DECLS_BEGIN_H

#ifdef __cplusplus
extern "C" {
#endif

#include "core.h"
#include <vector.h>

/* Text-shaped false-positive bait, inside the SAME ERROR subtree: this is
   not a directive (no `#`), and `phantom.h` is a real project file. A scan
   that read the ERROR region's TEXT would manufacture an edge to it; a walk
   that routes only grammar-labelled `preproc_include` nodes cannot. */
   include "phantom.h"

int decls_begin_marker(void);
