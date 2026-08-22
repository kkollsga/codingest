#ifndef UTILS_H
#define UTILS_H

/* The single most common real-world header shape (cJSON, unity, …): the
   `extern "C" {` brace is closed under a MATCHING `#ifdef` at the bottom of
   the file, so tree-sitter — which has no preprocessor — sees an unbalanced
   brace and parses everything below it as one ERROR recovery subtree. Every
   `#include` inside that subtree used to be invisible to the router. */
#ifdef __cplusplus
extern "C" {
#endif

#include "core.h"
#include <vector.h>

int utils_run(void);

#ifdef __cplusplus
}
#endif

#endif
