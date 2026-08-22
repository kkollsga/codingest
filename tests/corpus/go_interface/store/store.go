// Package store holds the demo key/value abstractions.
package store

// MaxEntries caps how much a store keeps.
const MaxEntries = 8

// Reader is the read side of a store.
type Reader interface {
	Fetch(key string) string
	Reset()
}

// Entry is one stored pair.
type Entry struct {
	Key   string
	Value string
}

// normalize cleans one stored value.
func normalize(value string) string {
	return value
}
