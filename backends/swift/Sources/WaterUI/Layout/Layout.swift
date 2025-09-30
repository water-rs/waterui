import CWaterUI
import SwiftUI

/*
 
 pub trait Layout {
     // Propose sizes for each child based on the parent's proposal and children's metadata.
     fn propose(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Vec<ProposalSize>;

     // These children will receive proposal by underlying renderer, then report their current proposal.
     fn size(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Size;

     // Place children within the given bounds and return their final rectangles.
     fn place(
         &mut self,
         bound: Rect,
         proposal: ProposalSize,
         children: &[ChildMetadata],
     ) -> Vec<Rect>;
 }
 
 */


class WuiLayout{
    var inner:OpaquePointer
    
    init(inner: OpaquePointer) {
        self.inner = inner
    }
    
    func propose(parent: WuiProposalSize, children: [WuiChildMetadata]) -> [WuiProposalSize] {
        fatalError()
    }
    
    func size(parent: WuiProposalSize, children: [WuiChildMetadata]) -> WuiSize {
        fatalError()
    }
    
    func place(bound: WuiRect, proposal: WuiProposalSize, children: [WuiChildMetadata]) -> [WuiRect] {
        fatalError()
    }
}

struct WuiContainer:WuiComponent,View{
    static var id = waterui_container_id()
    var layout:WuiLayout
    var children:[WuiAnyView]
    
    init(anyview: OpaquePointer, env: WuiEnvironment){
        let container = waterui_force_as_container(anyview)
        self.layout = .init(inner: container.layout!)
        var anyviews = WuiAnyViews(container.contents,env:env)
        self.children = anyviews.toArray()
    }

    
    var body: some View {
        fatalError()
    }
}
