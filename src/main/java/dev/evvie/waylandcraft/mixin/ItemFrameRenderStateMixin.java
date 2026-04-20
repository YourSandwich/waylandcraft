package dev.evvie.waylandcraft.mixin;

import org.spongepowered.asm.mixin.Mixin;

import dev.evvie.waylandcraft.bridge.WLCToplevel;
import dev.evvie.waylandcraft.render.IMyItemFrameRenderState;
import net.minecraft.client.renderer.entity.state.ItemFrameRenderState;

@Mixin(ItemFrameRenderState.class)
public class ItemFrameRenderStateMixin implements IMyItemFrameRenderState {
	
	public WLCToplevel toplevel;
	
	@Override
	public void setToplevel(WLCToplevel toplevel) {
		this.toplevel = toplevel;
	}
	
	@Override
	public WLCToplevel getToplevel() {
		return toplevel;
	}
	
}
