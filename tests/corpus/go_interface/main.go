package main

import (
	"fmt"

	"demo/store"
)

func main() {
	s := store.NewMemory()
	value := s.Fetch("k1")
	fmt.Println(value)
	missingHelper()
}
