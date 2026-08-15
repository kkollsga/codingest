# Bait target: this file's module collides with `library(tools)` in main.R.
# Nothing sources this file, so ANY File->File edge pointing here is a
# manufactured edge.
pad_left <- function(s, n) {
  formatC(s, width = n)
}
