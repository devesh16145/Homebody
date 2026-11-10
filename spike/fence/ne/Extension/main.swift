// Entry point for the system extension process. The system instantiates the principal
// class named in Info.plist (FenceExtension.FilterDataProvider) once we enter extension mode.

import Foundation
import NetworkExtension

autoreleasepool {
    NEProvider.startSystemExtensionMode()
}
dispatchMain()
