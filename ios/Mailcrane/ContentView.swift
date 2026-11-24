import SwiftUI

struct ContentView: View {
    @State private var viewModel = ContentViewModel(core: LiveCoreService())

    var body: some View {
        Text(viewModel.greeting)
            .padding()
    }
}
