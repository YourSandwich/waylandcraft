package dev.evvie.waylandcraft;

import org.joml.Quaternionf;
import org.joml.Vector3f;
import org.lwjgl.glfw.GLFW;

import net.minecraft.client.Minecraft;
import net.minecraft.util.Mth;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.phys.Vec3;

public class WaylandCraftUtils {
	
	public static Vec3 getPosition(Entity entity) {
		float partialTicks = Minecraft.getInstance().getDeltaTracker().getGameTimeDeltaPartialTick(true);
		Vec3 pos = new Vec3(
			Mth.lerp(partialTicks, entity.xo, entity.getX()),
			Mth.lerp(partialTicks, entity.yo, entity.getY()) + entity.getEyeHeight(),
			Mth.lerp(partialTicks, entity.zo, entity.getZ())
		);
		return pos;
	}
	
	public static Vec3 getLookVector(Entity entity) {
		float partialTicks = Minecraft.getInstance().getDeltaTracker().getGameTimeDeltaPartialTick(true);
		float yaw = entity.getViewYRot(partialTicks);
		float pitch = entity.getViewXRot(partialTicks);
		
		Quaternionf rotation = new Quaternionf();
		rotation.rotationYXZ(-yaw * Mth.PI / 180.0f, pitch * Mth.PI / 180.0f, 0.0f);
		
		Vec3 look = new Vec3(new Vector3f(0, 0, 1).rotate(rotation));
		return look;
	}
	
	public static Vec3 getUpVector(Entity entity) {
		float partialTicks = Minecraft.getInstance().getDeltaTracker().getGameTimeDeltaPartialTick(true);
		float yaw = entity.getViewYRot(partialTicks);
		float pitch = entity.getViewXRot(partialTicks);
		
		Quaternionf rotation = new Quaternionf();
		rotation.rotationYXZ(-yaw * Mth.PI / 180.0f, pitch * Mth.PI / 180.0f, 0.0f);
		
		Vec3 up = new Vec3(new Vector3f(0, 1, 0).rotate(rotation));
		return up;
	}

	public static boolean isAltHeld() {
		long handle = Minecraft.getInstance().getWindow().handle();
		return GLFW.glfwGetKey(handle, GLFW.GLFW_KEY_LEFT_ALT) == GLFW.GLFW_PRESS
				|| GLFW.glfwGetKey(handle, GLFW.GLFW_KEY_RIGHT_ALT) == GLFW.GLFW_PRESS;
	}

	public static boolean isControlHeld() {
		long handle = Minecraft.getInstance().getWindow().handle();
		return GLFW.glfwGetKey(handle, GLFW.GLFW_KEY_LEFT_CONTROL) == GLFW.GLFW_PRESS
				|| GLFW.glfwGetKey(handle, GLFW.GLFW_KEY_RIGHT_CONTROL) == GLFW.GLFW_PRESS;
	}

	public static boolean isShiftHeld() {
		long handle = Minecraft.getInstance().getWindow().handle();
		return GLFW.glfwGetKey(handle, GLFW.GLFW_KEY_LEFT_SHIFT) == GLFW.GLFW_PRESS
				|| GLFW.glfwGetKey(handle, GLFW.GLFW_KEY_RIGHT_SHIFT) == GLFW.GLFW_PRESS;
	}

}
