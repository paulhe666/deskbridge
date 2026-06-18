#import <AppKit/AppKit.h>
#import <dispatch/dispatch.h>
#include <stdint.h>

extern void deskbridge_handle_status_menu_command(const char *command);

static NSStatusItem *DeskbridgeStatusItem = nil;
static id DeskbridgeStatusTarget = nil;

@interface DeskbridgeStatusMenuTarget : NSObject
- (void)toggleRun:(id)sender;
- (void)quitDeskbridge:(id)sender;
@end

@implementation DeskbridgeStatusMenuTarget
- (void)toggleRun:(id)sender {
  deskbridge_handle_status_menu_command("toggle-run");
}

- (void)quitDeskbridge:(id)sender {
  deskbridge_handle_status_menu_command("quit");
}
@end

void deskbridge_install_status_item(const uint8_t *png_bytes, uintptr_t png_len) {
  if (png_bytes == NULL || png_len == 0) {
    return;
  }

  dispatch_async(dispatch_get_main_queue(), ^{
    if (DeskbridgeStatusItem != nil) {
      return;
    }

    NSData *data = [NSData dataWithBytes:png_bytes length:(NSUInteger)png_len];
    NSImage *image = [[NSImage alloc] initWithData:data];
    if (image == nil) {
      return;
    }

    image.size = NSMakeSize(21.0, 21.0);
    image.template = YES;
    DeskbridgeStatusTarget = [[DeskbridgeStatusMenuTarget alloc] init];

    DeskbridgeStatusItem =
        [[NSStatusBar systemStatusBar] statusItemWithLength:NSSquareStatusItemLength];
    DeskbridgeStatusItem.length = 28.0;
    DeskbridgeStatusItem.button.image = image;
    DeskbridgeStatusItem.button.imagePosition = NSImageOnly;
    DeskbridgeStatusItem.button.toolTip = @"Deskbridge";

    NSMenu *menu = [[NSMenu alloc] initWithTitle:@"Deskbridge"];
    NSMenuItem *title =
        [[NSMenuItem alloc] initWithTitle:@"Deskbridge" action:nil keyEquivalent:@""];
    title.enabled = NO;
    [menu addItem:title];
    [menu addItem:[NSMenuItem separatorItem]];

    NSMenuItem *toggle =
        [[NSMenuItem alloc] initWithTitle:@"启动/停止连接"
                                   action:@selector(toggleRun:)
                            keyEquivalent:@""];
    toggle.target = DeskbridgeStatusTarget;
    [menu addItem:toggle];

    NSMenuItem *quit =
        [[NSMenuItem alloc] initWithTitle:@"退出 Deskbridge"
                                   action:@selector(quitDeskbridge:)
                            keyEquivalent:@""];
    quit.target = DeskbridgeStatusTarget;
    [menu addItem:quit];

    DeskbridgeStatusItem.menu = menu;
  });
}
