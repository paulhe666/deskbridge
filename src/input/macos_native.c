#include "macos_native.h"

#include <ApplicationServices/ApplicationServices.h>
#include <Carbon/Carbon.h>
#include <CoreFoundation/CoreFoundation.h>
#include <IOKit/hid/IOHIDDeviceKeys.h>
#include <IOKit/hid/IOHIDManager.h>
#include <IOKit/hid/IOHIDUsageTables.h>
#include <pthread.h>
#include <stdlib.h>
#include <string.h>

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

typedef struct DeskbridgeEventTapState {
    void *context;
    DeskbridgeEventTapCallback callback;
    CFMachPortRef tap;
} DeskbridgeEventTapState;

static pthread_mutex_t db_event_tap_lock = PTHREAD_MUTEX_INITIALIZER;
static CFRunLoopRef db_event_tap_run_loop = NULL;
static bool db_event_tap_stop_requested = false;

static void db_set_event_tap_run_loop(CFRunLoopRef run_loop) {
    pthread_mutex_lock(&db_event_tap_lock);
    if (db_event_tap_run_loop != NULL) {
        CFRelease(db_event_tap_run_loop);
    }
    db_event_tap_run_loop = run_loop;
    if (db_event_tap_run_loop != NULL) {
        CFRetain(db_event_tap_run_loop);
    }
    pthread_mutex_unlock(&db_event_tap_lock);
}

static CFRunLoopRef db_copy_event_tap_run_loop(void) {
    pthread_mutex_lock(&db_event_tap_lock);
    CFRunLoopRef run_loop = db_event_tap_run_loop;
    if (run_loop != NULL) {
        CFRetain(run_loop);
    }
    pthread_mutex_unlock(&db_event_tap_lock);
    return run_loop;
}

static bool db_take_event_tap_stop_request(void) {
    pthread_mutex_lock(&db_event_tap_lock);
    bool requested = db_event_tap_stop_requested;
    db_event_tap_stop_requested = false;
    pthread_mutex_unlock(&db_event_tap_lock);
    return requested;
}

static int64_t db_event_int(CGEventRef event, CGEventField field) {
    return CGEventGetIntegerValueField(event, field);
}

static bool db_tap_emit_mouse(
    DeskbridgeEventTapState *state,
    uint32_t kind,
    CGEventRef event,
    int64_t button) {
    CGPoint point = CGEventGetLocation(event);
    int64_t dx = db_event_int(event, kCGMouseEventDeltaX);
    int64_t dy = db_event_int(event, kCGMouseEventDeltaY);
    int64_t flags = (int64_t)CGEventGetFlags(event);
    return state->callback(
        state->context, kind, button, dx, dy, flags, point.x, point.y);
}

static bool db_tap_emit_key(
    DeskbridgeEventTapState *state,
    uint32_t kind,
    CGEventRef event) {
    CGPoint point = CGEventGetLocation(event);
    int64_t keycode = db_event_int(event, kCGKeyboardEventKeycode);
    int64_t autorepeat = db_event_int(event, kCGKeyboardEventAutorepeat);
    uint64_t flags = (uint64_t)CGEventGetFlags(event);
    return state->callback(
        state->context,
        kind,
        keycode,
        autorepeat,
        (int64_t)flags,
        0,
        point.x,
        point.y);
}

static bool db_tap_emit_scroll(
    DeskbridgeEventTapState *state,
    CGEventRef event) {
    CGPoint point = CGEventGetLocation(event);
    int64_t vertical =
        db_event_int(event, kCGScrollWheelEventPointDeltaAxis1);
    int64_t horizontal =
        db_event_int(event, kCGScrollWheelEventPointDeltaAxis2);
    if (vertical == 0) {
        vertical = db_event_int(event, kCGScrollWheelEventDeltaAxis1);
    }
    if (horizontal == 0) {
        horizontal = db_event_int(event, kCGScrollWheelEventDeltaAxis2);
    }
    return state->callback(
        state->context,
        DESKBRIDGE_TAP_SCROLL,
        horizontal,
        vertical,
        0,
        0,
        point.x,
        point.y);
}

static CGEventRef db_event_tap_callback(
    CGEventTapProxy proxy,
    CGEventType type,
    CGEventRef event,
    void *refcon) {
    (void)proxy;
    DeskbridgeEventTapState *state = (DeskbridgeEventTapState *)refcon;
    if (state == NULL || state->callback == NULL) {
        return event;
    }

    if (type == kCGEventTapDisabledByTimeout ||
        type == kCGEventTapDisabledByUserInput) {
        if (state->tap != NULL) {
            CGEventTapEnable(state->tap, true);
        }
        return event;
    }

    bool suppress = false;
    switch (type) {
    case kCGEventMouseMoved:
    case kCGEventLeftMouseDragged:
    case kCGEventRightMouseDragged:
    case kCGEventOtherMouseDragged:
        suppress =
            db_tap_emit_mouse(state, DESKBRIDGE_MOUSE_MOVED, event, 0);
        break;
    case kCGEventLeftMouseDown:
        suppress = db_tap_emit_mouse(
            state,
            DESKBRIDGE_MOUSE_LEFT_DOWN,
            event,
            DESKBRIDGE_MOUSE_BUTTON_LEFT);
        break;
    case kCGEventLeftMouseUp:
        suppress = db_tap_emit_mouse(
            state,
            DESKBRIDGE_MOUSE_LEFT_UP,
            event,
            DESKBRIDGE_MOUSE_BUTTON_LEFT);
        break;
    case kCGEventRightMouseDown:
        suppress = db_tap_emit_mouse(
            state,
            DESKBRIDGE_MOUSE_RIGHT_DOWN,
            event,
            DESKBRIDGE_MOUSE_BUTTON_RIGHT);
        break;
    case kCGEventRightMouseUp:
        suppress = db_tap_emit_mouse(
            state,
            DESKBRIDGE_MOUSE_RIGHT_UP,
            event,
            DESKBRIDGE_MOUSE_BUTTON_RIGHT);
        break;
    case kCGEventOtherMouseDown:
        suppress = db_tap_emit_mouse(
            state,
            DESKBRIDGE_MOUSE_OTHER_DOWN,
            event,
            db_event_int(event, kCGMouseEventButtonNumber));
        break;
    case kCGEventOtherMouseUp:
        suppress = db_tap_emit_mouse(
            state,
            DESKBRIDGE_MOUSE_OTHER_UP,
            event,
            db_event_int(event, kCGMouseEventButtonNumber));
        break;
    case kCGEventScrollWheel:
        suppress = db_tap_emit_scroll(state, event);
        break;
    case kCGEventKeyDown:
        suppress = db_tap_emit_key(state, DESKBRIDGE_TAP_KEY_DOWN, event);
        break;
    case kCGEventKeyUp:
        suppress = db_tap_emit_key(state, DESKBRIDGE_TAP_KEY_UP, event);
        break;
    case kCGEventFlagsChanged:
        suppress =
            db_tap_emit_key(state, DESKBRIDGE_TAP_FLAGS_CHANGED, event);
        break;
    default:
        break;
    }

    return suppress ? NULL : event;
}

int32_t deskbridge_event_tap_run(
    void *context,
    DeskbridgeEventTapCallback callback) {
    if (callback == NULL) {
        return 1;
    }

    CGEventMask mask =
        CGEventMaskBit(kCGEventMouseMoved) |
        CGEventMaskBit(kCGEventLeftMouseDragged) |
        CGEventMaskBit(kCGEventRightMouseDragged) |
        CGEventMaskBit(kCGEventOtherMouseDragged) |
        CGEventMaskBit(kCGEventLeftMouseDown) |
        CGEventMaskBit(kCGEventLeftMouseUp) |
        CGEventMaskBit(kCGEventRightMouseDown) |
        CGEventMaskBit(kCGEventRightMouseUp) |
        CGEventMaskBit(kCGEventOtherMouseDown) |
        CGEventMaskBit(kCGEventOtherMouseUp) |
        CGEventMaskBit(kCGEventScrollWheel) |
        CGEventMaskBit(kCGEventKeyDown) |
        CGEventMaskBit(kCGEventKeyUp) |
        CGEventMaskBit(kCGEventFlagsChanged);

    DeskbridgeEventTapState state = {
        .context = context,
        .callback = callback,
        .tap = NULL,
    };
    state.tap = CGEventTapCreate(
        kCGHIDEventTap,
        kCGHeadInsertEventTap,
        kCGEventTapOptionDefault,
        mask,
        db_event_tap_callback,
        &state);
    if (state.tap == NULL) {
        return 2;
    }

    CFRunLoopSourceRef source =
        CFMachPortCreateRunLoopSource(kCFAllocatorDefault, state.tap, 0);
    if (source == NULL) {
        CFRelease(state.tap);
        return 3;
    }

    CFRunLoopRef run_loop = CFRunLoopGetCurrent();
    db_set_event_tap_run_loop(run_loop);
    CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
    CGEventTapEnable(state.tap, true);
    if (!db_take_event_tap_stop_request()) {
        CFRunLoopRun();
    }
    CFRunLoopRemoveSource(run_loop, source, kCFRunLoopCommonModes);
    db_set_event_tap_run_loop(NULL);
    CFRelease(source);
    CFRelease(state.tap);
    return 0;
}

void deskbridge_event_tap_stop(void) {
    CFRunLoopRef run_loop = db_copy_event_tap_run_loop();
    if (run_loop == NULL) {
        pthread_mutex_lock(&db_event_tap_lock);
        db_event_tap_stop_requested = true;
        pthread_mutex_unlock(&db_event_tap_lock);
        return;
    }
    CFRunLoopStop(run_loop);
    CFRunLoopWakeUp(run_loop);
    CFRelease(run_loop);
}

int32_t deskbridge_macos_set_cursor_position(double x, double y) {
    CGError error = CGWarpMouseCursorPosition(CGPointMake(x, y));
    return error == kCGErrorSuccess ? 0 : (int32_t)error;
}

int32_t deskbridge_macos_hide_cursor(void) {
    CGError error = CGDisplayHideCursor(CGMainDisplayID());
    CGAssociateMouseAndMouseCursorPosition(true);
    return error == kCGErrorSuccess ? 0 : (int32_t)error;
}

int32_t deskbridge_macos_show_cursor(void) {
    CGError error = CGDisplayShowCursor(CGMainDisplayID());
    CGAssociateMouseAndMouseCursorPosition(true);
    return error == kCGErrorSuccess ? 0 : (int32_t)error;
}

static bool db_cfboolean_is_true(CFTypeRef value) {
    return value != NULL && CFGetTypeID(value) == CFBooleanGetTypeID() &&
           CFBooleanGetValue((CFBooleanRef)value);
}

static bool db_input_source_is_keyboard(TISInputSourceRef source) {
    CFTypeRef category =
        TISGetInputSourceProperty(source, kTISPropertyInputSourceCategory);
    return category != NULL &&
           CFEqual(category, kTISCategoryKeyboardInputSource);
}

static bool db_input_source_is_selectable(TISInputSourceRef source) {
    return db_input_source_is_keyboard(source) &&
           db_cfboolean_is_true(TISGetInputSourceProperty(
               source, kTISPropertyInputSourceIsEnabled)) &&
           db_cfboolean_is_true(TISGetInputSourceProperty(
               source, kTISPropertyInputSourceIsSelectCapable));
}

static bool db_input_sources_equal(
    TISInputSourceRef left,
    TISInputSourceRef right) {
    if (left == NULL || right == NULL) {
        return false;
    }

    CFTypeRef left_id =
        TISGetInputSourceProperty(left, kTISPropertyInputSourceID);
    CFTypeRef right_id =
        TISGetInputSourceProperty(right, kTISPropertyInputSourceID);
    return left_id != NULL && right_id != NULL && CFEqual(left_id, right_id);
}

static bool db_cfstring_has_prefix(CFStringRef string, const char *prefix) {
    if (string == NULL || prefix == NULL) {
        return false;
    }

    char buffer[64] = {0};
    if (!CFStringGetCString(
            string,
            buffer,
            sizeof(buffer),
            kCFStringEncodingUTF8)) {
        return false;
    }

    return strncmp(buffer, prefix, strlen(prefix)) == 0;
}

static bool db_input_source_has_language_prefix(
    TISInputSourceRef source,
    const char *prefix) {
    CFTypeRef languages =
        TISGetInputSourceProperty(source, kTISPropertyInputSourceLanguages);
    if (languages == NULL || CFGetTypeID(languages) != CFArrayGetTypeID()) {
        return false;
    }

    CFArrayRef array = (CFArrayRef)languages;
    CFIndex count = CFArrayGetCount(array);
    for (CFIndex i = 0; i < count; i++) {
        CFTypeRef value = CFArrayGetValueAtIndex(array, i);
        if (value != NULL && CFGetTypeID(value) == CFStringGetTypeID() &&
            db_cfstring_has_prefix((CFStringRef)value, prefix)) {
            return true;
        }
    }
    return false;
}

static bool db_input_source_is_latin(TISInputSourceRef source) {
    return db_input_source_has_language_prefix(source, "en");
}

static bool db_input_source_is_chinese(TISInputSourceRef source) {
    return db_input_source_has_language_prefix(source, "zh");
}

static CFIndex db_preferred_toggle_source_index(
    CFArrayRef sources,
    TISInputSourceRef current) {
    if (sources == NULL || current == NULL) {
        return -1;
    }

    bool current_is_latin = db_input_source_is_latin(current);
    CFIndex count = CFArrayGetCount(sources);
    CFIndex fallback = -1;

    for (CFIndex i = 0; i < count; i++) {
        TISInputSourceRef source =
            (TISInputSourceRef)CFArrayGetValueAtIndex(sources, i);
        if (!db_input_source_is_selectable(source)) {
            continue;
        }

        bool source_is_latin = db_input_source_is_latin(source);
        if (current_is_latin) {
            if (db_input_source_is_chinese(source)) {
                return i;
            }
            if (!source_is_latin && fallback < 0) {
                fallback = i;
            }
        } else if (source_is_latin) {
            return i;
        }
    }

    return fallback;
}

int32_t deskbridge_hid_cycle_keyboard_input_source(DeskbridgeHidContext *context) {
    if (context == NULL) {
        return 1;
    }

    CFArrayRef sources = TISCreateInputSourceList(NULL, false);
    if (sources == NULL) {
        return 2;
    }

    TISInputSourceRef current = TISCopyCurrentKeyboardInputSource();
    CFIndex count = CFArrayGetCount(sources);
    CFIndex first_selectable = -1;
    CFIndex current_index = -1;

    for (CFIndex i = 0; i < count; i++) {
        TISInputSourceRef source =
            (TISInputSourceRef)CFArrayGetValueAtIndex(sources, i);
        if (!db_input_source_is_selectable(source)) {
            continue;
        }
        if (first_selectable < 0) {
            first_selectable = i;
        }
        if (db_input_sources_equal(source, current)) {
            current_index = i;
        }
    }

    if (first_selectable < 0) {
        if (current != NULL) {
            CFRelease(current);
        }
        CFRelease(sources);
        return 3;
    }

    CFIndex selected_index = db_preferred_toggle_source_index(sources, current);
    if (selected_index < 0) {
        selected_index = first_selectable;
        if (current_index >= 0) {
            for (CFIndex offset = 1; offset <= count; offset++) {
                CFIndex candidate = (current_index + offset) % count;
                TISInputSourceRef source =
                    (TISInputSourceRef)CFArrayGetValueAtIndex(sources, candidate);
                if (db_input_source_is_selectable(source)) {
                    selected_index = candidate;
                    break;
                }
            }
        }
    }

    if (current != NULL) {
        CFRelease(current);
    }

    TISInputSourceRef selected =
        (TISInputSourceRef)CFArrayGetValueAtIndex(sources, selected_index);
    OSStatus status = TISSelectInputSource(selected);
    CFRelease(sources);
    return status == noErr ? 0 : 4;
}

void deskbridge_main_display_size(uint32_t *width, uint32_t *height) {
    CGDirectDisplayID display = CGMainDisplayID();
    CGRect bounds = CGDisplayBounds(display);
    if (width != NULL) {
        *width = (uint32_t)CGRectGetWidth(bounds);
    }
    if (height != NULL) {
        *height = (uint32_t)CGRectGetHeight(bounds);
    }
}
