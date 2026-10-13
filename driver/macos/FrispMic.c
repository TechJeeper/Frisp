#include <CoreAudio/AudioServerPlugIn.h>
#include <mach/mach_time.h>
#include <stdatomic.h>
#include <string.h>

#define kDevice 2
#define kStreamOut 3
#define kStreamIn 4
#define kFrispBufferFrameSize 'fsiz'
#define kFrispBufferFrameSizeRange 'fsz#'
#define kSampleRate 48000.0
#define kRingCap 65536u
#define kRingMask (kRingCap - 1u)

static AudioServerPlugInDriverInterface gInterface;
static AudioServerPlugInDriverInterface *gInterfacePtr = &gInterface;
static AudioServerPlugInHostRef gHost = NULL;

static CFStringRef gDeviceName = NULL;
static CFStringRef gManufacturer = NULL;
static CFStringRef gUID = NULL;
static CFStringRef gModelUID = NULL;
static CFStringRef gStreamInName = NULL;
static CFStringRef gStreamOutName = NULL;
static CFStringRef gVersion = NULL;

static Float32 gRing[kRingCap * 2];
static atomic_uint gWritePos = 0;
static atomic_uint gReadPos = 0;
static atomic_int gIOCount = 0;
static atomic_uint gBufferFrames = 512;
static atomic_ullong gSeed = 1;
static UInt64 gAnchorHostTime = 0;
static Float64 gHostTicksPerFrame = 1.0;

static void init_strings(void) {
    if (gDeviceName != NULL) {
        return;
    }
    gDeviceName = CFStringCreateWithCString(NULL, "Frisp Mic", kCFStringEncodingUTF8);
    gManufacturer = CFStringCreateWithCString(NULL, "Frisp", kCFStringEncodingUTF8);
    gUID = CFStringCreateWithCString(NULL, "com.frisp.mic", kCFStringEncodingUTF8);
    gModelUID = CFStringCreateWithCString(NULL, "Frisp Mic", kCFStringEncodingUTF8);
    gStreamInName = CFStringCreateWithCString(NULL, "Frisp Mic Input", kCFStringEncodingUTF8);
    gStreamOutName = CFStringCreateWithCString(NULL, "Frisp Mic Output", kCFStringEncodingUTF8);
    gVersion = CFStringCreateWithCString(NULL, "0.1.0", kCFStringEncodingUTF8);
}

static void init_clock(void) {
    struct mach_timebase_info info;
    mach_timebase_info(&info);
    Float64 ticksPerSecond = 1e9 * ((Float64)info.denom / (Float64)info.numer);
    gHostTicksPerFrame = ticksPerSecond / kSampleRate;
}

static OSStatus write_bytes(const void *src, UInt32 srcSize, UInt32 inSize, UInt32 *outSize, void *outData) {
    if (outSize != NULL) {
        *outSize = srcSize;
    }
    if (outData != NULL) {
        if (inSize < srcSize) {
            return kAudioHardwareBadPropertySizeError;
        }
        memcpy(outData, src, srcSize);
    }
    return noErr;
}

static OSStatus write_string(CFStringRef value, UInt32 inSize, UInt32 *outSize, void *outData) {
    UInt32 size = (UInt32)sizeof(CFStringRef);
    if (outSize != NULL) {
        *outSize = size;
    }
    if (outData != NULL) {
        if (inSize < size) {
            return kAudioHardwareBadPropertySizeError;
        }
        *((CFStringRef *)outData) = CFStringCreateCopy(NULL, value);
    }
    return noErr;
}

static AudioStreamBasicDescription stereo_format(void) {
    AudioStreamBasicDescription format;
    memset(&format, 0, sizeof(format));
    format.mSampleRate = kSampleRate;
    format.mFormatID = kAudioFormatLinearPCM;
    format.mFormatFlags = kAudioFormatFlagIsFloat | kAudioFormatFlagsNativeEndian | kAudioFormatFlagIsPacked;
    format.mBitsPerChannel = 32;
    format.mChannelsPerFrame = 2;
    format.mFramesPerPacket = 1;
    format.mBytesPerFrame = 8;
    format.mBytesPerPacket = 8;
    return format;
}

static void sample_from(const AudioBufferList *abl, UInt32 frame, Float32 *left, Float32 *right) {
    *left = 0;
    *right = 0;
    if (abl == NULL || abl->mNumberBuffers == 0) {
        return;
    }
    if (abl->mNumberBuffers == 1) {
        const AudioBuffer *buffer = &abl->mBuffers[0];
        if (buffer->mData == NULL || buffer->mDataByteSize == 0) {
            return;
        }
        UInt32 channels = buffer->mNumberChannels == 0 ? 2 : buffer->mNumberChannels;
        UInt32 maxFrames = buffer->mDataByteSize / (sizeof(Float32) * channels);
        if (frame >= maxFrames) {
            return;
        }
        const Float32 *src = (const Float32 *)buffer->mData;
        *left = src[frame * channels];
        *right = channels > 1 ? src[frame * channels + 1] : *left;
        return;
    }
    if (abl->mBuffers[0].mData != NULL) {
        UInt32 maxFrames = abl->mBuffers[0].mDataByteSize / sizeof(Float32);
        if (frame < maxFrames) {
            *left = ((const Float32 *)abl->mBuffers[0].mData)[frame];
        }
    }
    const AudioBuffer *rightBuffer = &abl->mBuffers[abl->mNumberBuffers > 1 ? 1 : 0];
    if (rightBuffer->mData != NULL) {
        UInt32 maxFrames = rightBuffer->mDataByteSize / sizeof(Float32);
        if (frame < maxFrames) {
            *right = ((const Float32 *)rightBuffer->mData)[frame];
        }
    } else {
        *right = *left;
    }
}

static void sample_to(AudioBufferList *abl, UInt32 frame, Float32 left, Float32 right) {
    if (abl == NULL || abl->mNumberBuffers == 0) {
        return;
    }
    if (abl->mNumberBuffers == 1) {
        AudioBuffer *buffer = &abl->mBuffers[0];
        if (buffer->mData == NULL) {
            return;
        }
        UInt32 channels = buffer->mNumberChannels == 0 ? 2 : buffer->mNumberChannels;
        UInt32 maxFrames = buffer->mDataByteSize / (sizeof(Float32) * channels);
        if (frame >= maxFrames) {
            return;
        }
        Float32 *dst = (Float32 *)buffer->mData;
        dst[frame * channels] = left;
        if (channels > 1) {
            dst[frame * channels + 1] = right;
        }
        return;
    }
    if (abl->mBuffers[0].mData != NULL) {
        UInt32 maxFrames = abl->mBuffers[0].mDataByteSize / sizeof(Float32);
        if (frame < maxFrames) {
            ((Float32 *)abl->mBuffers[0].mData)[frame] = left;
        }
    }
    if (abl->mNumberBuffers > 1 && abl->mBuffers[1].mData != NULL) {
        UInt32 maxFrames = abl->mBuffers[1].mDataByteSize / sizeof(Float32);
        if (frame < maxFrames) {
            ((Float32 *)abl->mBuffers[1].mData)[frame] = right;
        }
    }
}

static void ring_write(const AudioBufferList *abl, UInt32 frames) {
    if (frames == 0) {
        return;
    }
    uint32_t write = atomic_load_explicit(&gWritePos, memory_order_relaxed);
    uint32_t read = atomic_load_explicit(&gReadPos, memory_order_acquire);
    uint32_t used = write - read;
    if (used + frames >= kRingCap) {
        uint32_t drop = used + frames - (kRingCap - 1u);
        atomic_store_explicit(&gReadPos, read + drop, memory_order_release);
    }
    for (UInt32 i = 0; i < frames; i++) {
        Float32 left, right;
        sample_from(abl, i, &left, &right);
        uint32_t pos = (write + i) & kRingMask;
        gRing[pos * 2] = left;
        gRing[pos * 2 + 1] = right;
    }
    atomic_store_explicit(&gWritePos, write + frames, memory_order_release);
}

static void ring_read(AudioBufferList *abl, UInt32 frames) {
    if (abl != NULL) {
        for (UInt32 i = 0; i < abl->mNumberBuffers; i++) {
            if (abl->mBuffers[i].mData != NULL && abl->mBuffers[i].mDataByteSize > 0) {
                memset(abl->mBuffers[i].mData, 0, abl->mBuffers[i].mDataByteSize);
            }
        }
    }
    uint32_t write = atomic_load_explicit(&gWritePos, memory_order_acquire);
    uint32_t read = atomic_load_explicit(&gReadPos, memory_order_relaxed);
    if (write - read < frames) {
        return;
    }
    for (UInt32 i = 0; i < frames; i++) {
        uint32_t pos = (read + i) & kRingMask;
        sample_to(abl, i, gRing[pos * 2], gRing[pos * 2 + 1]);
    }
    atomic_store_explicit(&gReadPos, read + frames, memory_order_release);
}

static Boolean uuid_is(REFIID uuid, CFUUIDRef reference) {
    CFUUIDBytes expected = CFUUIDGetUUIDBytes(reference);
    return memcmp(&uuid, &expected, sizeof(expected)) == 0;
}

static HRESULT QueryInterface(void *inDriver, REFIID inUUID, LPVOID *outInterface) {
    (void)inDriver;
    if (outInterface == NULL) {
        return E_INVALIDARG;
    }
    if (uuid_is(inUUID, IUnknownUUID) || uuid_is(inUUID, kAudioServerPlugInDriverInterfaceUUID)) {
        *outInterface = &gInterfacePtr;
        return S_OK;
    }
    return E_NOINTERFACE;
}

static ULONG AddRef(void *inDriver) {
    (void)inDriver;
    return 1;
}

static ULONG Release(void *inDriver) {
    (void)inDriver;
    return 1;
}

static OSStatus Initialize(AudioServerPlugInDriverRef inDriver, AudioServerPlugInHostRef inHost) {
    (void)inDriver;
    gHost = inHost;
    init_strings();
    init_clock();
    return noErr;
}

static OSStatus CreateDevice(AudioServerPlugInDriverRef inDriver, CFDictionaryRef inDescription,
                             const AudioServerPlugInClientInfo *inClientInfo, AudioObjectID *outDeviceObjectID) {
    (void)inDriver;
    (void)inDescription;
    (void)inClientInfo;
    (void)outDeviceObjectID;
    return kAudioHardwareUnsupportedOperationError;
}

static OSStatus DestroyDevice(AudioServerPlugInDriverRef inDriver, AudioObjectID inDeviceObjectID) {
    (void)inDriver;
    (void)inDeviceObjectID;
    return kAudioHardwareUnsupportedOperationError;
}

static OSStatus AddDeviceClient(AudioServerPlugInDriverRef inDriver, AudioObjectID inDeviceObjectID,
                                const AudioServerPlugInClientInfo *inClientInfo) {
    (void)inDriver;
    (void)inDeviceObjectID;
    (void)inClientInfo;
    return noErr;
}

static OSStatus RemoveDeviceClient(AudioServerPlugInDriverRef inDriver, AudioObjectID inDeviceObjectID,
                                   const AudioServerPlugInClientInfo *inClientInfo) {
    (void)inDriver;
    (void)inDeviceObjectID;
    (void)inClientInfo;
    return noErr;
}

static OSStatus PerformDeviceConfigurationChange(AudioServerPlugInDriverRef inDriver, AudioObjectID inDeviceObjectID,
                                                 UInt64 inChangeAction, void *inChangeInfo) {
    (void)inDriver;
    (void)inDeviceObjectID;
    (void)inChangeAction;
    (void)inChangeInfo;
    return noErr;
}

static OSStatus AbortDeviceConfigurationChange(AudioServerPlugInDriverRef inDriver, AudioObjectID inDeviceObjectID,
                                               UInt64 inChangeAction, void *inChangeInfo) {
    (void)inDriver;
    (void)inDeviceObjectID;
    (void)inChangeAction;
    (void)inChangeInfo;
    return noErr;
}

static UInt32 stream_list(AudioObjectPropertyScope scope, AudioObjectID *ids) {
    if (scope == kAudioObjectPropertyScopeInput) {
        ids[0] = kStreamIn;
        return 1;
    }
    if (scope == kAudioObjectPropertyScopeOutput) {
        ids[0] = kStreamOut;
        return 1;
    }
    ids[0] = kStreamOut;
    ids[1] = kStreamIn;
    return 2;
}

static Boolean HasProperty(AudioServerPlugInDriverRef inDriver, AudioObjectID inObjectID, pid_t inClientProcessID,
                           const AudioObjectPropertyAddress *inAddress) {
    (void)inDriver;
    (void)inClientProcessID;
    if (inAddress == NULL) {
        return false;
    }
    AudioObjectPropertySelector selector = inAddress->mSelector;
    if (selector == kAudioObjectPropertyBaseClass || selector == kAudioObjectPropertyClass ||
        selector == kAudioObjectPropertyOwner || selector == kAudioObjectPropertyName ||
        selector == kAudioObjectPropertyManufacturer || selector == kAudioObjectPropertyOwnedObjects ||
        selector == kAudioObjectPropertySerialNumber || selector == kAudioObjectPropertyFirmwareVersion) {
        return true;
    }
    if (inObjectID == kAudioObjectPlugInObject) {
        return selector == kAudioPlugInPropertyDeviceList || selector == kAudioPlugInPropertyTranslateUIDToDevice;
    }
    if (inObjectID == kDevice) {
        switch (selector) {
        case kAudioDevicePropertyDeviceUID:
        case kAudioDevicePropertyModelUID:
        case kAudioDevicePropertyTransportType:
        case kAudioDevicePropertyRelatedDevices:
        case kAudioDevicePropertyClockDomain:
        case kAudioDevicePropertyDeviceIsAlive:
        case kAudioDevicePropertyDeviceIsRunning:
        case kAudioDevicePropertyDeviceCanBeDefaultDevice:
        case kAudioDevicePropertyDeviceCanBeDefaultSystemDevice:
        case kAudioDevicePropertyLatency:
        case kAudioDevicePropertyStreams:
        case kAudioDevicePropertySafetyOffset:
        case kAudioDevicePropertyNominalSampleRate:
        case kAudioDevicePropertyAvailableNominalSampleRates:
        case kAudioDevicePropertyIsHidden:
        case kAudioDevicePropertyPreferredChannelsForStereo:
        case kAudioDevicePropertyPreferredChannelLayout:
        case kAudioDevicePropertyZeroTimeStampPeriod:
        case kFrispBufferFrameSize:
        case kFrispBufferFrameSizeRange:
        case kAudioObjectPropertyIdentify:
            return true;
        default:
            return false;
        }
    }
    if (inObjectID == kStreamIn || inObjectID == kStreamOut) {
        switch (selector) {
        case kAudioStreamPropertyIsActive:
        case kAudioStreamPropertyDirection:
        case kAudioStreamPropertyTerminalType:
        case kAudioStreamPropertyStartingChannel:
        case kAudioStreamPropertyLatency:
        case kAudioStreamPropertyVirtualFormat:
        case kAudioStreamPropertyPhysicalFormat:
        case kAudioStreamPropertyAvailableVirtualFormats:
        case kAudioStreamPropertyAvailablePhysicalFormats:
            return true;
        default:
            return false;
        }
    }
    return false;
}

static OSStatus IsPropertySettable(AudioServerPlugInDriverRef inDriver, AudioObjectID inObjectID, pid_t inClientProcessID,
                                   const AudioObjectPropertyAddress *inAddress, Boolean *outIsSettable) {
    if (!HasProperty(inDriver, inObjectID, inClientProcessID, inAddress)) {
        return kAudioHardwareUnknownPropertyError;
    }
    if (outIsSettable == NULL) {
        return kAudioHardwareIllegalOperationError;
    }
    AudioObjectPropertySelector selector = inAddress->mSelector;
    *outIsSettable = selector == kAudioObjectPropertyIdentify || selector == kAudioDevicePropertyNominalSampleRate ||
                     selector == kFrispBufferFrameSize;
    return noErr;
}

static OSStatus GetPropertyDataSize(AudioServerPlugInDriverRef inDriver, AudioObjectID inObjectID, pid_t inClientProcessID,
                                    const AudioObjectPropertyAddress *inAddress, UInt32 inQualifierDataSize,
                                    const void *inQualifierData, UInt32 *outDataSize) {
    (void)inQualifierDataSize;
    (void)inQualifierData;
    if (!HasProperty(inDriver, inObjectID, inClientProcessID, inAddress) || outDataSize == NULL) {
        return kAudioHardwareUnknownPropertyError;
    }
    switch (inAddress->mSelector) {
    case kAudioObjectPropertyName:
    case kAudioObjectPropertyManufacturer:
    case kAudioObjectPropertySerialNumber:
    case kAudioObjectPropertyFirmwareVersion:
    case kAudioDevicePropertyDeviceUID:
    case kAudioDevicePropertyModelUID:
    case kAudioPlugInPropertyTranslateUIDToDevice:
        *outDataSize = (UInt32)sizeof(CFStringRef);
        if (inAddress->mSelector == kAudioPlugInPropertyTranslateUIDToDevice) {
            *outDataSize = (UInt32)sizeof(AudioObjectID);
        }
        return noErr;
    case kAudioObjectPropertyOwnedObjects:
        if (inObjectID == kAudioObjectPlugInObject) {
            *outDataSize = (UInt32)sizeof(AudioObjectID);
        } else if (inObjectID == kDevice) {
            *outDataSize = (UInt32)(sizeof(AudioObjectID) * 2);
        } else {
            *outDataSize = 0;
        }
        return noErr;
    case kAudioDevicePropertyStreams: {
        AudioObjectID ids[2];
        *outDataSize = stream_list(inAddress->mScope, ids) * (UInt32)sizeof(AudioObjectID);
        return noErr;
    }
    case kAudioPlugInPropertyDeviceList:
    case kAudioDevicePropertyRelatedDevices:
        *outDataSize = (UInt32)sizeof(AudioObjectID);
        return noErr;
    case kAudioDevicePropertyPreferredChannelsForStereo:
        *outDataSize = (UInt32)(sizeof(UInt32) * 2);
        return noErr;
    case kAudioDevicePropertyPreferredChannelLayout:
        *outDataSize = (UInt32)offsetof(AudioChannelLayout, mChannelDescriptions);
        return noErr;
    case kAudioDevicePropertyNominalSampleRate:
        *outDataSize = (UInt32)sizeof(Float64);
        return noErr;
    case kAudioDevicePropertyAvailableNominalSampleRates:
        *outDataSize = (UInt32)sizeof(AudioValueRange);
        return noErr;
    case kFrispBufferFrameSizeRange:
        *outDataSize = (UInt32)sizeof(AudioValueRange);
        return noErr;
    case kAudioStreamPropertyVirtualFormat:
    case kAudioStreamPropertyPhysicalFormat:
        *outDataSize = (UInt32)sizeof(AudioStreamBasicDescription);
        return noErr;
    case kAudioStreamPropertyAvailableVirtualFormats:
    case kAudioStreamPropertyAvailablePhysicalFormats:
        *outDataSize = (UInt32)sizeof(AudioStreamRangedDescription);
        return noErr;
    default:
        *outDataSize = (UInt32)sizeof(UInt32);
        return noErr;
    }
}

static OSStatus GetPropertyData(AudioServerPlugInDriverRef inDriver, AudioObjectID inObjectID, pid_t inClientProcessID,
                                const AudioObjectPropertyAddress *inAddress, UInt32 inQualifierDataSize,
                                const void *inQualifierData, UInt32 inDataSize, UInt32 *outDataSize, void *outData) {
    if (!HasProperty(inDriver, inObjectID, inClientProcessID, inAddress)) {
        return kAudioHardwareUnknownPropertyError;
    }
    AudioObjectPropertySelector selector = inAddress->mSelector;
    AudioObjectPropertyScope scope = inAddress->mScope;

    if (selector == kAudioObjectPropertyBaseClass) {
        AudioClassID value = kAudioObjectClassID;
        return write_bytes(&value, sizeof(value), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioObjectPropertyClass) {
        AudioClassID value = kAudioObjectClassID;
        if (inObjectID == kAudioObjectPlugInObject) {
            value = kAudioPlugInClassID;
        } else if (inObjectID == kDevice) {
            value = kAudioDeviceClassID;
        } else {
            value = kAudioStreamClassID;
        }
        return write_bytes(&value, sizeof(value), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioObjectPropertyOwner) {
        AudioObjectID owner = kAudioObjectUnknown;
        if (inObjectID == kDevice) {
            owner = kAudioObjectPlugInObject;
        } else if (inObjectID == kStreamIn || inObjectID == kStreamOut) {
            owner = kDevice;
        }
        return write_bytes(&owner, sizeof(owner), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioObjectPropertyName) {
        CFStringRef name = gDeviceName;
        if (inObjectID == kStreamIn) {
            name = gStreamInName;
        } else if (inObjectID == kStreamOut) {
            name = gStreamOutName;
        }
        return write_string(name, inDataSize, outDataSize, outData);
    }
    if (selector == kAudioObjectPropertyManufacturer) {
        return write_string(gManufacturer, inDataSize, outDataSize, outData);
    }
    if (selector == kAudioObjectPropertySerialNumber || selector == kAudioObjectPropertyFirmwareVersion) {
        return write_string(gVersion, inDataSize, outDataSize, outData);
    }
    if (selector == kAudioObjectPropertyOwnedObjects) {
        AudioObjectID ids[2];
        UInt32 count = 0;
        if (inObjectID == kAudioObjectPlugInObject) {
            ids[0] = kDevice;
            count = 1;
        } else if (inObjectID == kDevice) {
            ids[0] = kStreamOut;
            ids[1] = kStreamIn;
            count = 2;
        }
        return write_bytes(ids, count * (UInt32)sizeof(AudioObjectID), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioPlugInPropertyDeviceList || selector == kAudioDevicePropertyRelatedDevices) {
        AudioObjectID id = kDevice;
        return write_bytes(&id, sizeof(id), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioPlugInPropertyTranslateUIDToDevice) {
        AudioObjectID id = kAudioObjectUnknown;
        if (inQualifierDataSize >= sizeof(CFStringRef) && inQualifierData != NULL) {
            CFStringRef uid = *(CFStringRef *)inQualifierData;
            if (uid != NULL && CFStringCompare(uid, gUID, 0) == kCFCompareEqualTo) {
                id = kDevice;
            }
        }
        return write_bytes(&id, sizeof(id), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioDevicePropertyDeviceUID) {
        return write_string(gUID, inDataSize, outDataSize, outData);
    }
    if (selector == kAudioDevicePropertyModelUID) {
        return write_string(gModelUID, inDataSize, outDataSize, outData);
    }
    if (selector == kAudioDevicePropertyTransportType) {
        UInt32 value = kAudioDeviceTransportTypeVirtual;
        return write_bytes(&value, sizeof(value), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioDevicePropertyClockDomain || selector == kAudioDevicePropertySafetyOffset ||
        selector == kAudioDevicePropertyIsHidden || selector == kAudioObjectPropertyIdentify) {
        UInt32 value = 0;
        return write_bytes(&value, sizeof(value), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioDevicePropertyDeviceIsAlive) {
        UInt32 value = 1;
        return write_bytes(&value, sizeof(value), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioDevicePropertyDeviceIsRunning || selector == kAudioStreamPropertyIsActive) {
        UInt32 value = atomic_load(&gIOCount) > 0 ? 1 : 0;
        if (selector == kAudioStreamPropertyIsActive) {
            value = 1;
        }
        return write_bytes(&value, sizeof(value), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioDevicePropertyDeviceCanBeDefaultDevice ||
        selector == kAudioDevicePropertyDeviceCanBeDefaultSystemDevice) {
        UInt32 value = 0;
        if (scope != kAudioObjectPropertyScopeOutput) {
            value = 1;
        }
        if (selector == kAudioDevicePropertyDeviceCanBeDefaultSystemDevice && scope == kAudioObjectPropertyScopeOutput) {
            value = 0;
        }
        return write_bytes(&value, sizeof(value), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioDevicePropertyLatency) {
        UInt32 value = atomic_load(&gBufferFrames);
        return write_bytes(&value, sizeof(value), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioStreamPropertyLatency || selector == kAudioStreamPropertyStartingChannel) {
        UInt32 value = selector == kAudioStreamPropertyStartingChannel ? 1 : 0;
        return write_bytes(&value, sizeof(value), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioDevicePropertyStreams) {
        AudioObjectID ids[2];
        UInt32 count = stream_list(scope, ids);
        return write_bytes(ids, count * (UInt32)sizeof(AudioObjectID), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioDevicePropertyNominalSampleRate) {
        Float64 value = kSampleRate;
        return write_bytes(&value, sizeof(value), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioDevicePropertyAvailableNominalSampleRates || selector == kFrispBufferFrameSizeRange) {
        AudioValueRange range;
        if (selector == kFrispBufferFrameSizeRange) {
            range.mMinimum = 128;
            range.mMaximum = 2048;
        } else {
            range.mMinimum = kSampleRate;
            range.mMaximum = kSampleRate;
        }
        return write_bytes(&range, sizeof(range), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioDevicePropertyPreferredChannelsForStereo) {
        UInt32 channels[2] = {1, 2};
        return write_bytes(channels, sizeof(channels), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioDevicePropertyPreferredChannelLayout) {
        AudioChannelLayout layout;
        memset(&layout, 0, sizeof(layout));
        layout.mChannelLayoutTag = kAudioChannelLayoutTag_Stereo;
        return write_bytes(&layout, (UInt32)offsetof(AudioChannelLayout, mChannelDescriptions), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioDevicePropertyZeroTimeStampPeriod || selector == kFrispBufferFrameSize) {
        UInt32 value = atomic_load(&gBufferFrames);
        return write_bytes(&value, sizeof(value), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioStreamPropertyDirection) {
        UInt32 value = inObjectID == kStreamIn ? 1 : 0;
        return write_bytes(&value, sizeof(value), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioStreamPropertyTerminalType) {
        UInt32 value = inObjectID == kStreamIn ? kAudioStreamTerminalTypeMicrophone : kAudioStreamTerminalTypeLine;
        return write_bytes(&value, sizeof(value), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioStreamPropertyVirtualFormat || selector == kAudioStreamPropertyPhysicalFormat) {
        AudioStreamBasicDescription format = stereo_format();
        return write_bytes(&format, sizeof(format), inDataSize, outDataSize, outData);
    }
    if (selector == kAudioStreamPropertyAvailableVirtualFormats || selector == kAudioStreamPropertyAvailablePhysicalFormats) {
        AudioStreamRangedDescription format;
        memset(&format, 0, sizeof(format));
        format.mFormat = stereo_format();
        format.mSampleRateRange.mMinimum = kSampleRate;
        format.mSampleRateRange.mMaximum = kSampleRate;
        return write_bytes(&format, sizeof(format), inDataSize, outDataSize, outData);
    }
    return kAudioHardwareUnknownPropertyError;
}

static OSStatus SetPropertyData(AudioServerPlugInDriverRef inDriver, AudioObjectID inObjectID, pid_t inClientProcessID,
                                const AudioObjectPropertyAddress *inAddress, UInt32 inQualifierDataSize,
                                const void *inQualifierData, UInt32 inDataSize, const void *inData) {
    (void)inQualifierDataSize;
    (void)inQualifierData;
    Boolean settable = false;
    OSStatus status = IsPropertySettable(inDriver, inObjectID, inClientProcessID, inAddress, &settable);
    if (status != noErr) {
        return status;
    }
    if (!settable) {
        return kAudioHardwareUnsupportedOperationError;
    }
    if (inAddress->mSelector == kAudioObjectPropertyIdentify) {
        return noErr;
    }
    if (inData == NULL) {
        return kAudioHardwareIllegalOperationError;
    }
    if (inAddress->mSelector == kAudioDevicePropertyNominalSampleRate) {
        if (inDataSize < sizeof(Float64)) {
            return kAudioHardwareBadPropertySizeError;
        }
        Float64 rate = *(const Float64 *)inData;
        if (rate < kSampleRate - 1.0 || rate > kSampleRate + 1.0) {
            return kAudioHardwareUnsupportedOperationError;
        }
        return noErr;
    }
    if (inAddress->mSelector == kFrispBufferFrameSize) {
        if (inDataSize < sizeof(UInt32)) {
            return kAudioHardwareBadPropertySizeError;
        }
        UInt32 frames = *(const UInt32 *)inData;
        if (frames < 128 || frames > 2048) {
            return kAudioHardwareUnsupportedOperationError;
        }
        atomic_store(&gBufferFrames, frames);
        if (gHost != NULL && gHost->PropertiesChanged != NULL) {
            AudioObjectPropertyAddress address = {kFrispBufferFrameSize, kAudioObjectPropertyScopeGlobal,
                                                  kAudioObjectPropertyElementMain};
            gHost->PropertiesChanged(gHost, kDevice, 1, &address);
        }
        return noErr;
    }
    return kAudioHardwareUnknownPropertyError;
}

static OSStatus StartIO(AudioServerPlugInDriverRef inDriver, AudioObjectID inDeviceObjectID, UInt32 inClientID) {
    (void)inDriver;
    (void)inDeviceObjectID;
    (void)inClientID;
    int previous = atomic_fetch_add(&gIOCount, 1);
    if (previous == 0) {
        gAnchorHostTime = mach_absolute_time();
        atomic_fetch_add(&gSeed, 1);
    }
    return noErr;
}

static OSStatus StopIO(AudioServerPlugInDriverRef inDriver, AudioObjectID inDeviceObjectID, UInt32 inClientID) {
    (void)inDriver;
    (void)inDeviceObjectID;
    (void)inClientID;
    int count = atomic_load(&gIOCount);
    if (count > 0) {
        atomic_fetch_sub(&gIOCount, 1);
    }
    return noErr;
}

static OSStatus GetZeroTimeStamp(AudioServerPlugInDriverRef inDriver, AudioObjectID inDeviceObjectID, UInt32 inClientID,
                                 Float64 *outSampleTime, UInt64 *outHostTime, UInt64 *outSeed) {
    (void)inDriver;
    (void)inDeviceObjectID;
    (void)inClientID;
    if (atomic_load(&gIOCount) <= 0 || outSampleTime == NULL || outHostTime == NULL || outSeed == NULL) {
        return kAudioHardwareNotRunningError;
    }
    UInt32 bufferFrames = atomic_load(&gBufferFrames);
    if (bufferFrames == 0) {
        bufferFrames = 512;
    }
    UInt64 now = mach_absolute_time();
    Float64 elapsed = (Float64)(now - gAnchorHostTime);
    Float64 frames = elapsed / gHostTicksPerFrame;
    UInt64 quantized = ((UInt64)frames / bufferFrames) * bufferFrames;
    *outSampleTime = (Float64)quantized;
    *outHostTime = gAnchorHostTime + (UInt64)((Float64)quantized * gHostTicksPerFrame);
    *outSeed = atomic_load(&gSeed);
    return noErr;
}

static OSStatus WillDoIOOperation(AudioServerPlugInDriverRef inDriver, AudioObjectID inDeviceObjectID, UInt32 inClientID,
                                  UInt32 inOperationID, Boolean *outWillDo, Boolean *outWillDoInPlace) {
    (void)inDriver;
    (void)inDeviceObjectID;
    (void)inClientID;
    if (outWillDo == NULL || outWillDoInPlace == NULL) {
        return kAudioHardwareIllegalOperationError;
    }
    *outWillDo = inOperationID == kAudioServerPlugInIOOperationReadInput ||
                 inOperationID == kAudioServerPlugInIOOperationWriteMix;
    *outWillDoInPlace = true;
    return noErr;
}

static OSStatus BeginIOOperation(AudioServerPlugInDriverRef inDriver, AudioObjectID inDeviceObjectID, UInt32 inClientID,
                                 UInt32 inOperationID, UInt32 inIOBufferFrameSize,
                                 const AudioServerPlugInIOCycleInfo *inIOCycleInfo) {
    (void)inDriver;
    (void)inDeviceObjectID;
    (void)inClientID;
    (void)inOperationID;
    (void)inIOBufferFrameSize;
    (void)inIOCycleInfo;
    return noErr;
}

static OSStatus DoIOOperation(AudioServerPlugInDriverRef inDriver, AudioObjectID inDeviceObjectID,
                              AudioObjectID inStreamObjectID, UInt32 inClientID, UInt32 inOperationID,
                              UInt32 inIOBufferFrameSize, const AudioServerPlugInIOCycleInfo *inIOCycleInfo,
                              void *ioMainBuffer, void *ioSecondaryBuffer) {
    (void)inDriver;
    (void)inDeviceObjectID;
    (void)inStreamObjectID;
    (void)inClientID;
    (void)inIOCycleInfo;
    (void)ioSecondaryBuffer;
    if (inOperationID == kAudioServerPlugInIOOperationWriteMix) {
        ring_write((const AudioBufferList *)ioMainBuffer, inIOBufferFrameSize);
    } else if (inOperationID == kAudioServerPlugInIOOperationReadInput) {
        ring_read((AudioBufferList *)ioMainBuffer, inIOBufferFrameSize);
    }
    return noErr;
}

static OSStatus EndIOOperation(AudioServerPlugInDriverRef inDriver, AudioObjectID inDeviceObjectID, UInt32 inClientID,
                               UInt32 inOperationID, UInt32 inIOBufferFrameSize,
                               const AudioServerPlugInIOCycleInfo *inIOCycleInfo) {
    (void)inDriver;
    (void)inDeviceObjectID;
    (void)inClientID;
    (void)inOperationID;
    (void)inIOBufferFrameSize;
    (void)inIOCycleInfo;
    return noErr;
}

static AudioServerPlugInDriverInterface gInterface = {
    NULL,
    QueryInterface,
    AddRef,
    Release,
    Initialize,
    CreateDevice,
    DestroyDevice,
    AddDeviceClient,
    RemoveDeviceClient,
    PerformDeviceConfigurationChange,
    AbortDeviceConfigurationChange,
    HasProperty,
    IsPropertySettable,
    GetPropertyDataSize,
    GetPropertyData,
    SetPropertyData,
    StartIO,
    StopIO,
    GetZeroTimeStamp,
    WillDoIOOperation,
    BeginIOOperation,
    DoIOOperation,
    EndIOOperation,
};

__attribute__((visibility("default"))) void *FrispMic_Create(CFAllocatorRef inAllocator, CFUUIDRef inRequestedTypeUUID) {
    (void)inAllocator;
    if (inRequestedTypeUUID != NULL && CFEqual(inRequestedTypeUUID, kAudioServerPlugInTypeUUID)) {
        return &gInterfacePtr;
    }
    return NULL;
}
