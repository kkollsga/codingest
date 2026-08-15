# B of the source chain: sources C relative to this file's own directory.
source("deep.R")

# S4 class with representation slots.
setClass("Person", representation(name = "character", age = "numeric"))

setGeneric("greet", function(object) standardGeneric("greet"))

# S4 method: attaches to Person via HAS_METHOD.
setMethod("greet", "Person", function(object) {
  describe(object@name)
})

describe <- function(label) {
  paste("hello", label)
}
