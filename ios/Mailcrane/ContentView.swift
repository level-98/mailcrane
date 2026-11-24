import SwiftUI

struct ContentView: View {
    @State private var viewModel = ContentViewModel(core: LiveCoreService())

    var body: some View {
        ZStack {
            Color.brandSurface
                .ignoresSafeArea()
            Text(viewModel.greeting)
                .foregroundStyle(Color.brandInk)
                .padding()
        }
    }
}
