package dev.evvie.waylandcraft.render;

import net.irisshaders.iris.api.v0.IrisApi;

// Isolates every reference to the Iris API. This class is only ever loaded when
// the "iris" mod is present (callers guard with isModLoaded), so WaylandCraft
// still runs fine without Iris installed.
public class IrisCompat {

	public static boolean isShaderPackActive() {
		return IrisApi.getInstance().isShaderPackInUse();
	}

	public static boolean isShadowPass() {
		return IrisApi.getInstance().isRenderingShadowPass();
	}

}
