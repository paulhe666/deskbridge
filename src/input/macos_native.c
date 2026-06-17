#include "macos_native.h"

#include <ApplicationServices/ApplicationServices.h>
#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/hid/IOHIDDeviceKeys.h>
#include <IOKit/hid/IOHIDManager.h>
#include <IOKit/hid/IOHIDUsageTables.h>
#include <stdlib.h>

struct DeskbridgeHidContext {
    CGEventSourceRef source;
    IOHIDManagerRef hid_manager;
};

static CFNumberRef db_number_i32(int32_t value) {
    return CFNumberCreate(kCFAllocatorDefault, kCFNumberSInt32Type, &value);
}

static CFDictionaryRef db_usage_match(int32_t usage_page, int32_t usage) {
    CFNumberRef page = db_number_i32(usage_page);
    CFNumberRef use = db_number_i32(usage);
    if (page == NULL || use == NULL) {
        if (page != NULL) {
            CFRelease(page);
        }
        if (use != NULL) {
            CFRelease(use);
        }
        return NULL;
    }

    const void *keys[] = {
        CFSTR(kIOHIDDeviceUsagePageKey),
        CFSTR(kIOHIDDeviceUsageKey),
    };
    const void *values[] = {page, use};
    CFDictionaryRef dictionary = CFDictionaryCreate(
        kCFAllocatorDefault,
        keys,
        values,
        2,
        &kCFCopyStringDictionaryKeyCallBacks,
        &kCFTypeDictionaryValueCallBacks);
    CFRelease(page);
    CFRelease(use);
    return dictionary;
}

static CFArrayRef db_usage_matches(void) {
    CFDictionaryRef keyboard =
        db_usage_match(kHIDPage_GenericDesktop, kHIDUsage_GD_Keyboard);
    CFDictionaryRef mouse =
        db_usage_match(kHIDPage_GenericDesktop, kHIDUsage_GD_Mouse);
    if (keyboard == NULL || mouse == NULL) {
        if (keyboard != NULL) {
            CFRelease(keyboard);
        }
        if (mouse != NULL) {
            CFRelease(mouse);
        }
        return NULL;
    }

    const void *values[] = {keyboard, mouse};
    CFArrayRef matches = CFArrayCreate(
        kCFAllocatorDefault,
        values,
        2,
        &kCFTypeArrayCallBacks);
    CFRelease(keyboard);
    CFRelease(mouse);
    return matches;
}

static bool db_number_matches(CFTypeRef value, int32_t expected) {
    if (value == NULL || CFGetTypeID(value) != CFNumberGetTypeID()) {
        return false;
    }

    int32_t actual = 0;
    if (!CFNumberGetValue((CFNumberRef)value, kCFNumberSInt32Type, &actual)) {
        return false;
    }
    return actual == expected;
}

static bool db_dictionary_matches_usage(
    CFDictionaryRef dictionary,
    int32_t usage_page,
    int32_t usage) {
    CFTypeRef page =
        CFDictionaryGetValue(dictionary, CFSTR(kIOHIDDeviceUsagePageKey));
    CFTypeRef use =
        CFDictionaryGetValue(dictionary, CFSTR(kIOHIDDeviceUsageKey));
    return db_number_matches(page, usage_page) && db_number_matches(use, usage);
}

static bool db_device_matches_usage(
    IOHIDDeviceRef device,
    int32_t usage_page,
    int32_t usage) {
    CFTypeRef pairs =
        IOHIDDeviceGetProperty(device, CFSTR(kIOHIDDeviceUsagePairsKey));
    if (pairs != NULL && CFGetTypeID(pairs) == CFArrayGetTypeID()) {
        CFArrayRef array = (CFArrayRef)pairs;
        CFIndex count = CFArrayGetCount(array);
        for (CFIndex i = 0; i < count; i++) {
            CFTypeRef value = CFArrayGetValueAtIndex(array, i);
            if (value == NULL || CFGetTypeID(value) != CFDictionaryGetTypeID()) {
                continue;
            }
            if (db_dictionary_matches_usage(
                    (CFDictionaryRef)value, usage_page, usage)) {
                return true;
            }
        }
    }

    CFTypeRef page =
        IOHIDDeviceGetProperty(device, CFSTR(kIOHIDDeviceUsagePageKey));
    CFTypeRef use = IOHIDDeviceGetProperty(device, CFSTR(kIOHIDDeviceUsageKey));
    return db_number_matches(page, usage_page) && db_number_matches(use, usage);
}

static void db_count_devices(
    IOHIDManagerRef manager,
    size_t *keyboard_count,
    size_t *mouse_count) {
    if (keyboard_count != NULL) {
        *keyboard_count = 0;
    }
    if (mouse_count != NULL) {
        *mouse_count = 0;
    }
    if (manager == NULL) {
        return;
    }

    CFSetRef devices = IOHIDManagerCopyDevices(manager);
    if (devices == NULL) {
        return;
    }

    CFIndex count = CFSetGetCount(devices);
    if (count <= 0) {
        CFRelease(devices);
        return;
    }

    const void **values = calloc((size_t)count, sizeof(void *));
    if (values == NULL) {
        CFRelease(devices);
        return;
    }

    CFSetGetValues(devices, values);
    for (CFIndex i = 0; i < count; i++) {
        IOHIDDeviceRef device = (IOHIDDeviceRef)values[i];
        if (keyboard_count != NULL &&
            db_device_matches_usage(
                device, kHIDPage_GenericDesktop, kHIDUsage_GD_Keyboard)) {
            (*keyboard_count)++;
        }
        if (mouse_count != NULL &&
            db_device_matches_usage(
                device, kHIDPage_GenericDesktop, kHIDUsage_GD_Mouse)) {
            (*mouse_count)++;
        }
    }

    free(values);
    CFRelease(devices);
}

static IOHIDManagerRef db_create_hid_manager(
    size_t *keyboard_count,
    size_t *mouse_count) {
    IOHIDManagerRef manager =
        IOHIDManagerCreate(kCFAllocatorDefault, kIOHIDManagerOptionNone);
    if (manager == NULL) {
        return NULL;
    }

    CFArrayRef matches = db_usage_matches();
    if (matches != NULL) {
        IOHIDManagerSetDeviceMatchingMultiple(manager, matches);
        CFRelease(matches);
    }

    if (IOHIDManagerOpen(manager, kIOHIDOptionsTypeNone) != kIOReturnSuccess) {
        CFRelease(manager);
        return NULL;
    }

    db_count_devices(manager, keyboard_count, mouse_count);
    return manager;
}

int32_t deskbridge_hid_context_create(
    DeskbridgeHidContext **context,
    size_t *keyboard_count,
    size_t *mouse_count) {
    if (context == NULL) {
        return 1;
    }
    *context = NULL;
    if (keyboard_count != NULL) {
        *keyboard_count = 0;
    }
    if (mouse_count != NULL) {
        *mouse_count = 0;
    }

    DeskbridgeHidContext *created = calloc(1, sizeof(DeskbridgeHidContext));
    if (created == NULL) {
        return 1;
    }

    created->source = CGEventSourceCreate(kCGEventSourceStateHIDSystemState);
    if (created->source == NULL) {
        free(created);
        return 1;
    }

    CGEventSourceSetLocalEventsSuppressionInterval(created->source, 0.0);
    CGEventSourceSetPixelsPerLine(created->source, 1.0);
    CGEventSourceSetUserData(created->source, 0x4445534b42524944LL);

    created->hid_manager = db_create_hid_manager(keyboard_count, mouse_count);
    *context = created;

    return created->hid_manager == NULL ? 2 : 0;
}

void deskbridge_hid_context_destroy(DeskbridgeHidContext *context) {
    if (context == NULL) {
        return;
    }
    if (context->hid_manager != NULL) {
        IOHIDManagerClose(context->hid_manager, kIOHIDOptionsTypeNone);
        CFRelease(context->hid_manager);
    }
    if (context->source != NULL) {
        CFRelease(context->source);
    }
    free(context);
}

int32_t deskbridge_hid_post_key(
    DeskbridgeHidContext *context,
    uint16_t keycode,
    bool down,
    uint64_t flags,
    bool autorepeat) {
    if (context == NULL || context->source == NULL) {
        return 1;
    }

    CGEventRef event =
        CGEventCreateKeyboardEvent(context->source, (CGKeyCode)keycode, down);
    if (event == NULL) {
        return 2;
    }

    CGEventSetFlags(event, (CGEventFlags)flags);
    CGEventSetIntegerValueField(
        event, kCGKeyboardEventAutorepeat, autorepeat ? 1 : 0);
    CGEventPost(kCGHIDEventTap, event);
    CFRelease(event);
    return 0;
}

static CGMouseButton db_mouse_button(uint8_t button) {
    switch (button) {
    case DESKBRIDGE_MOUSE_BUTTON_RIGHT:
        return kCGMouseButtonRight;
    case DESKBRIDGE_MOUSE_BUTTON_CENTER:
        return kCGMouseButtonCenter;
    case DESKBRIDGE_MOUSE_BUTTON_LEFT:
    default:
        return kCGMouseButtonLeft;
    }
}

static bool db_mouse_event_type(uint8_t kind, CGEventType *event_type) {
    switch (kind) {
    case DESKBRIDGE_MOUSE_MOVED:
        *event_type = kCGEventMouseMoved;
        return true;
    case DESKBRIDGE_MOUSE_LEFT_DOWN:
        *event_type = kCGEventLeftMouseDown;
        return true;
    case DESKBRIDGE_MOUSE_LEFT_UP:
        *event_type = kCGEventLeftMouseUp;
        return true;
    case DESKBRIDGE_MOUSE_RIGHT_DOWN:
        *event_type = kCGEventRightMouseDown;
        return true;
    case DESKBRIDGE_MOUSE_RIGHT_UP:
        *event_type = kCGEventRightMouseUp;
        return true;
    case DESKBRIDGE_MOUSE_OTHER_DOWN:
        *event_type = kCGEventOtherMouseDown;
        return true;
    case DESKBRIDGE_MOUSE_OTHER_UP:
        *event_type = kCGEventOtherMouseUp;
        return true;
    case DESKBRIDGE_MOUSE_LEFT_DRAGGED:
        *event_type = kCGEventLeftMouseDragged;
        return true;
    case DESKBRIDGE_MOUSE_RIGHT_DRAGGED:
        *event_type = kCGEventRightMouseDragged;
        return true;
    case DESKBRIDGE_MOUSE_OTHER_DRAGGED:
        *event_type = kCGEventOtherMouseDragged;
        return true;
    default:
        return false;
    }
}

int32_t deskbridge_hid_post_mouse(
    DeskbridgeHidContext *context,
    uint8_t kind,
    uint8_t button,
    double x,
    double y,
    int64_t click_count,
    int32_t dx,
    int32_t dy) {
    if (context == NULL || context->source == NULL) {
        return 1;
    }

    CGEventType event_type = kCGEventNull;
    if (!db_mouse_event_type(kind, &event_type)) {
        return 2;
    }

    CGPoint point = CGPointMake(x, y);
    CGMouseButton mouse_button = db_mouse_button(button);
    CGEventRef event =
        CGEventCreateMouseEvent(context->source, event_type, point, mouse_button);
    if (event == NULL) {
        return 3;
    }

    CGEventSetIntegerValueField(event, kCGMouseEventClickState, click_count);
    CGEventSetIntegerValueField(event, kCGMouseEventDeltaX, dx);
    CGEventSetIntegerValueField(event, kCGMouseEventDeltaY, dy);
    CGEventPost(kCGHIDEventTap, event);
    CFRelease(event);
    return 0;
}

int32_t deskbridge_hid_post_scroll(
    DeskbridgeHidContext *context,
    int32_t horizontal,
    int32_t vertical) {
    if (context == NULL || context->source == NULL) {
        return 1;
    }

    CGEventRef event = CGEventCreateScrollWheelEvent2(
        context->source,
        kCGScrollEventUnitPixel,
        2,
        vertical,
        horizontal,
        0);
    if (event == NULL) {
        return 2;
    }

    CGEventSetIntegerValueField(event, kCGScrollWheelEventIsContinuous, 1);
    CGEventSetIntegerValueField(
        event, kCGScrollWheelEventPointDeltaAxis1, vertical);
    CGEventSetIntegerValueField(
        event, kCGScrollWheelEventPointDeltaAxis2, horizontal);
    CGEventPost(kCGHIDEventTap, event);
    CFRelease(event);
    return 0;
}

void deskbridge_main_display_size(uint32_t *width, uint32_t *height) {
    CGDirectDisplayID display = CGMainDisplayID();
    if (width != NULL) {
        *width = (uint32_t)CGDisplayPixelsWide(display);
    }
    if (height != NULL) {
        *height = (uint32_t)CGDisplayPixelsHigh(display);
    }
}
