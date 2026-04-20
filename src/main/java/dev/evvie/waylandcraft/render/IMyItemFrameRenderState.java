package dev.evvie.waylandcraft.render;

import dev.evvie.waylandcraft.bridge.WLCToplevel;

public interface IMyItemFrameRenderState {
	
	void setToplevel(WLCToplevel toplevel);
	WLCToplevel getToplevel();
	
}
