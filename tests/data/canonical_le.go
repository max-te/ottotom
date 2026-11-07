//go:build ignore

// Generates canonical_le.txt, the reference for ottotom's canonical `le`
// label values: Go's shortest %g rendering, with ".0" appended when there is
// neither a decimal point nor an exponent, as the OpenMetrics spec targets.
//
//	go run tests/data/canonical_le.go > tests/data/canonical_le.txt
//
// Each line holds one canonical rendering. It is the shortest that reads back
// as the same f64, so the line alone identifies the value.
package main

import (
	"fmt"
	"math"
	"strconv"
	"strings"
)

func canonical(v float64) string {
	s := strconv.FormatFloat(v, 'g', -1, 64)
	if !strings.ContainsAny(s, ".eIN") {
		s += ".0"
	}
	return s
}

func main() {
	var values []float64
	add := func(v float64) {
		values = append(values, v, -v)
	}
	// Powers of ten, across the switch between fixed and exponent form.
	for exp := -20; exp <= 25; exp++ {
		add(math.Pow10(exp))
	}
	// The thresholds of the exponent form and their neighbours.
	for _, v := range []float64{0.0001, 0.00001, 999999, 1e6, 123456, 1234567} {
		add(v)
		add(math.Nextafter(v, 0))
		add(math.Nextafter(v, math.Inf(1)))
	}
	// The SDK's default histogram buckets.
	for _, v := range []float64{0, 5, 10, 25, 50, 75, 100, 250, 500, 750, 1000, 2500, 5000, 7500, 10000} {
		add(v)
	}
	// Extremes.
	for _, v := range []float64{5e-324, 2.2250738585072014e-308, math.MaxFloat64} {
		add(v)
	}
	// The spec's examples: 0.0 to 10.0 in steps of 0.001.
	for i := 0; i <= 10000; i++ {
		values = append(values, float64(i)/1000)
	}
	// Deterministic pseudo-random bit patterns: normal, subnormal, negative.
	state := uint64(0x9e3779b97f4a7c15)
	for random := 0; random < 1000; {
		state ^= state << 13
		state ^= state >> 7
		state ^= state << 17
		v := math.Float64frombits(state)
		if !math.IsNaN(v) && !math.IsInf(v, 0) {
			values = append(values, v)
			random++
		}
	}
	for _, v := range values {
		fmt.Println(canonical(v))
	}
}
