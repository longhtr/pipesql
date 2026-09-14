/* macOS controller for isolated Linux diagnostics and full-storage verification.
 * Verification refuses a weaker data-disk policy before starting the VM. */
#import <Foundation/Foundation.h>
#import <Virtualization/Virtualization.h>

@interface ProbeDelegate : NSObject <VZVirtualMachineDelegate>
@end

@implementation ProbeDelegate
- (void)guestDidStopVirtualMachine:(VZVirtualMachine *)machine {
    (void)machine;
    exit(0); // The supervisor must also validate the guest's completion records.
}

- (void)virtualMachine:(VZVirtualMachine *)machine didStopWithError:(NSError *)error {
    (void)machine;
    NSLog(@"VM failed: %@", error);
    exit(2);
}
@end

int main(int argc, const char **argv) {
    @autoreleasepool {
        if (argc != 6) {
            fprintf(stderr, "usage: host KERNEL BOOT_DISK NEW_DATA_DISK "
                    "full|fsync raw|catalog|verify|verify-smoke|verify-fail\n");
            return 2;
        }
        VZDiskImageSynchronizationMode mode;
        if (strcmp(argv[4], "full") == 0) {
            mode = VZDiskImageSynchronizationModeFull;
        } else if (strcmp(argv[4], "fsync") == 0) {
            mode = VZDiskImageSynchronizationModeFsync;
        } else {
            return 2;
        }
        BOOL verification = strcmp(argv[5], "verify") == 0
            || strcmp(argv[5], "verify-smoke") == 0 || strcmp(argv[5], "verify-fail") == 0;
        if (verification && mode != VZDiskImageSynchronizationModeFull) {
            fprintf(stderr, "verification requires full synchronization\n");
            return 2;
        }
        if (!verification && strcmp(argv[5], "raw") != 0 && strcmp(argv[5], "catalog") != 0) {
            return 2;
        }

        VZVirtualMachineConfiguration *configuration = [VZVirtualMachineConfiguration new];
        configuration.CPUCount = 1;
        configuration.memorySize = (verification ? 2048ULL : 256ULL) * 1024 * 1024;
        // Avoid measuring a newly booted kernel's wait for its first random seed.
        configuration.entropyDevices = @[[[VZVirtioEntropyDeviceConfiguration alloc] init]];
        VZLinuxBootLoader *boot = [[VZLinuxBootLoader alloc]
            initWithKernelURL:[NSURL fileURLWithPath:@(argv[1])]];
        boot.commandLine = [NSString stringWithFormat:
            @"console=hvc0 quiet panic=0 root=/dev/vda ro init=/init -- %@", @(argv[5])];
        configuration.bootLoader = boot;

        VZVirtioConsoleDeviceSerialPortConfiguration *serial =
            [[VZVirtioConsoleDeviceSerialPortConfiguration alloc] init];
        serial.attachment = [[VZFileHandleSerialPortAttachment alloc]
            initWithFileHandleForReading:nil
            fileHandleForWriting:[NSFileHandle fileHandleWithStandardOutput]];
        configuration.serialPorts = @[serial];

        NSError *error = nil;
        VZDiskImageStorageDeviceAttachment *root = [[VZDiskImageStorageDeviceAttachment alloc]
            initWithURL:[NSURL fileURLWithPath:@(argv[2])]
            readOnly:YES cachingMode:VZDiskImageCachingModeCached
            synchronizationMode:VZDiskImageSynchronizationModeFull error:&error];
        if (root == nil) {
            NSLog(@"Boot disk: %@", error);
            return 2;
        }
        VZDiskImageStorageDeviceAttachment *data = [[VZDiskImageStorageDeviceAttachment alloc]
            initWithURL:[NSURL fileURLWithPath:@(argv[3])]
            readOnly:NO cachingMode:VZDiskImageCachingModeCached
            synchronizationMode:mode error:&error];
        if (data == nil) {
            NSLog(@"Data disk: %@", error);
            return 2;
        }
        configuration.storageDevices = @[
            [[VZVirtioBlockDeviceConfiguration alloc] initWithAttachment:root],
            [[VZVirtioBlockDeviceConfiguration alloc] initWithAttachment:data]
        ];
        if (![configuration validateWithError:&error]) {
            NSLog(@"Configuration: %@", error);
            return 2;
        }
        NSLog(@"EXPERIMENT CPU=%lu memory=%llu cache=%ld sync=%ld network=%lu",
              (unsigned long)configuration.CPUCount,
              (unsigned long long)configuration.memorySize,
              (long)data.cachingMode, (long)data.synchronizationMode,
              (unsigned long)configuration.networkDevices.count);

        // Keep both alive until shutdown; the VM's delegate reference is weak.
        NS_VALID_UNTIL_END_OF_SCOPE ProbeDelegate *delegate = [ProbeDelegate new];
        NS_VALID_UNTIL_END_OF_SCOPE VZVirtualMachine *machine =
            [[VZVirtualMachine alloc] initWithConfiguration:configuration];
        machine.delegate = delegate;
        [machine startWithCompletionHandler:^(NSError *startError) {
            if (startError) {
                NSLog(@"Start: %@", startError);
                exit(2);
            }
        }];
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, (verification ? 7200LL : 60LL) * NSEC_PER_SEC),
                       dispatch_get_main_queue(), ^{
            NSLog(@"VM timeout");
            exit(3);
        });
        [[NSRunLoop mainRunLoop] run];
    }
    return 2;
}
