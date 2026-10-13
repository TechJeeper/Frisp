SDK := $(shell xcrun --show-sdk-path)
CC := clang
ARCHS ?= -arch x86_64 -arch arm64
CFLAGS := -bundle $(ARCHS) -isysroot $(SDK) -mmacosx-version-min=12.0 -fno-common -O2 -Wall -Wextra -Wno-unused-parameter
FRAMEWORKS := -framework CoreAudio -framework CoreFoundation
BUNDLE := build/FrispMic.driver

all: $(BUNDLE)/Contents/MacOS/FrispMic
	cp Info.plist $(BUNDLE)/Contents/Info.plist
	codesign --force --sign - $(BUNDLE)

$(BUNDLE)/Contents/MacOS/FrispMic: FrispMic.c Info.plist
	mkdir -p $(BUNDLE)/Contents/MacOS
	$(CC) $(CFLAGS) $(FRAMEWORKS) FrispMic.c -o $@

clean:
	rm -rf build

.PHONY: all clean
