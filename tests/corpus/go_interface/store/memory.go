package store

import "strings"

// Memory is an in-process Reader.
type Memory struct {
	entries map[string]string
}

// NewMemory builds an empty Memory.
func NewMemory() *Memory {
	return &Memory{entries: map[string]string{}}
}

// Fetch returns the normalized value for key.
func (m *Memory) Fetch(key string) string {
	return normalize(strings.TrimSpace(m.entries[key]))
}

// Reset drops every entry.
func (m *Memory) Reset() {
	m.entries = map[string]string{}
}
