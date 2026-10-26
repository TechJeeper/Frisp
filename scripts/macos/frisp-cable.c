/* Names a BlackHole virtual cable "Frisp Mic", the same way the Windows
   installer names VB-Audio Virtual Cable. */
#include <CoreAudio/CoreAudio.h>
#include <CoreFoundation/CoreFoundation.h>
#include <ctype.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char *kFrispName = "Frisp Mic";
static const char *kFrispUID = "com.frisp.mic";

static int contains_ci(const char *haystack, const char *needle) {
    size_t needle_len = strlen(needle);
    if (needle_len == 0) {
        return 1;
    }
    for (const char *cursor = haystack; *cursor; cursor++) {
        size_t index = 0;
        while (index < needle_len && cursor[index] &&
               tolower((unsigned char)cursor[index]) == tolower((unsigned char)needle[index])) {
            index++;
        }
        if (index == needle_len) {
            return 1;
        }
    }
    return 0;
}

static int copy_device_string(AudioObjectID device, AudioObjectPropertySelector selector, char *buffer, size_t length) {
    AudioObjectPropertyAddress address = {
        selector,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain
    };
    CFStringRef value = NULL;
    UInt32 size = sizeof(value);
    OSStatus status = AudioObjectGetPropertyData(device, &address, 0, NULL, &size, &value);
    if (status != noErr || value == NULL) {
        return 0;
    }
    Boolean ok = CFStringGetCString(value, buffer, (CFIndex)length, kCFStringEncodingUTF8);
    CFRelease(value);
    return ok ? 1 : 0;
}

static AudioObjectID *copy_devices(UInt32 *count) {
    AudioObjectPropertyAddress address = {
        kAudioHardwarePropertyDevices,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain
    };
    UInt32 size = 0;
    if (AudioObjectGetPropertyDataSize(kAudioObjectSystemObject, &address, 0, NULL, &size) != noErr || size == 0) {
        *count = 0;
        return NULL;
    }
    AudioObjectID *devices = malloc(size);
    if (devices == NULL) {
        *count = 0;
        return NULL;
    }
    if (AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, 0, NULL, &size, devices) != noErr) {
        free(devices);
        *count = 0;
        return NULL;
    }
    *count = size / sizeof(AudioObjectID);
    return devices;
}

static void missing_cable(void) {
    fprintf(stderr, "BlackHole 2ch is not installed.\n");
    fprintf(stderr, "Download it from https://existential.audio/blackhole/ , install it, then run this script again.\n");
}

int main(void) {
    UInt32 count = 0;
    AudioObjectID *devices = copy_devices(&count);
    char blackhole_uid[512];
    int blackhole_rank = 100;
    blackhole_uid[0] = '\0';

    for (UInt32 index = 0; index < count; index++) {
        char name[256];
        char uid[512];
        if (!copy_device_string(devices[index], kAudioObjectPropertyName, name, sizeof(name))) {
            continue;
        }
        if (!copy_device_string(devices[index], kAudioDevicePropertyDeviceUID, uid, sizeof(uid))) {
            continue;
        }
        if (strcmp(uid, kFrispUID) == 0) {
            printf("Frisp Mic is ready. Reopen Frisp, then choose Frisp Mic in OBS or your browser.\n");
            free(devices);
            return 0;
        }
        if (!contains_ci(name, "blackhole")) {
            continue;
        }
        int rank = contains_ci(name, "2ch") ? 0 : 1;
        if (rank < blackhole_rank) {
            blackhole_rank = rank;
            snprintf(blackhole_uid, sizeof(blackhole_uid), "%s", uid);
        }
    }
    free(devices);

    if (blackhole_uid[0] == '\0') {
        missing_cable();
        return 1;
    }

    CFMutableDictionaryRef description = CFDictionaryCreateMutable(
        NULL, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CFMutableDictionaryRef subdevice = CFDictionaryCreateMutable(
        NULL, 0, &kCFTypeDictionaryKeyCallBacks, &kCFTypeDictionaryValueCallBacks);
    CFMutableArrayRef subdevices = CFArrayCreateMutable(NULL, 1, &kCFTypeArrayCallBacks);
    CFStringRef name = CFStringCreateWithCString(NULL, kFrispName, kCFStringEncodingUTF8);
    CFStringRef uid = CFStringCreateWithCString(NULL, kFrispUID, kCFStringEncodingUTF8);
    CFStringRef subdevice_uid = CFStringCreateWithCString(NULL, blackhole_uid, kCFStringEncodingUTF8);
    int visible = 0;
    CFNumberRef private_flag = CFNumberCreate(NULL, kCFNumberIntType, &visible);

    CFDictionarySetValue(subdevice, CFSTR(kAudioSubDeviceUIDKey), subdevice_uid);
    CFArrayAppendValue(subdevices, subdevice);
    CFDictionarySetValue(description, CFSTR(kAudioAggregateDeviceNameKey), name);
    CFDictionarySetValue(description, CFSTR(kAudioAggregateDeviceUIDKey), uid);
    CFDictionarySetValue(description, CFSTR(kAudioAggregateDeviceSubDeviceListKey), subdevices);
    CFDictionarySetValue(description, CFSTR(kAudioAggregateDeviceMasterSubDeviceKey), subdevice_uid);
    CFDictionarySetValue(description, CFSTR(kAudioAggregateDeviceIsPrivateKey), private_flag);
    CFDictionarySetValue(description, CFSTR(kAudioAggregateDeviceIsStackedKey), private_flag);

    AudioObjectID created = 0;
    OSStatus status = AudioHardwareCreateAggregateDevice(description, &created);

    CFRelease(private_flag);
    CFRelease(subdevice_uid);
    CFRelease(uid);
    CFRelease(name);
    CFRelease(subdevices);
    CFRelease(subdevice);
    CFRelease(description);

    if (status != noErr || created == 0) {
        fprintf(stderr, "Could not name the virtual cable Frisp Mic (error %d).\n", (int)status);
        return 1;
    }

    printf("Named the virtual cable Frisp Mic.\n");
    printf("Frisp Mic is ready. Reopen Frisp, then choose Frisp Mic in OBS or your browser.\n");
    return 0;
}
