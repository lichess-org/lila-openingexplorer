all: stat convert

stat: stat.cc
	g++ -I/usr/include stat.cc -lkyotocabinet -o stat

convert: convert.cc
	g++ --std=c++11 -I/usr/include convert.cc -lkyotocabinet -o convert
